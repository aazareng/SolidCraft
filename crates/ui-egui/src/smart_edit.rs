//! Smart Edit: one key that edits whatever is picked, the way it most likely wants editing.
//!
//! - In a sketch: finish it.
//! - A face: the sketch of the feature that made it (its profile sketch), else that feature's
//!   dialog. A face of another part than the active one is edited in its placement.
//! - An edge: the same for the latest feature that made a face beside it (a hole's rim on a top
//!   face: the hole, through its points sketch), in the part's placement.
//! - A feature in the browser: its sketch, or the sketch itself; else its dialog.
//! - A plane: a new sketch on it.
//! - A part in an assembly: Edit in Place.
//! - Nothing picked: the sketch edited last (else the newest sketch of the active part); a part
//!   with no sketch yet gets a new one on its origin plane facing the screen.

use solidcraft_engine::Sel;
use solidcraft_engine::doc::FeatureKind;
use solidcraft_engine::geom::prelude::*;

use crate::SolveApp;

/// One press of the key.
pub fn press(app: &mut SolveApp) {
    if let Err(why) = step(app) {
        app.set_status(why, true);
    }
}

fn step(app: &mut SolveApp) -> Result<(), String> {
    if app.session.active_sketch.is_some() {
        app.finish_sketch();
        return Ok(());
    }
    if app.dialog.is_some() {
        return Err("Smart Edit: finish the open dialog first".into());
    }
    let sel = app.session.selection.clone();
    match sel.as_slice() {
        [] => nothing_picked(app),
        [Sel::Face { body, index, point }] => face(app, body, *index, *point),
        [Sel::Edge { body, index, point }] => edge(app, body, *index, *point),
        [Sel::Feature { id }] => {
            feature(app, *id, None);
            Ok(())
        }
        [Sel::Plane { .. }] => {
            app.start("sketch.create");
            Ok(())
        }
        [Sel::Occurrence { path }] => {
            let component = path
                .last()
                .and_then(|o| app.session.doc.occurrences.iter().find(|x| x.id == *o))
                .map(|o| o.component)
                .ok_or("the part is gone")?;
            crate::edit_in_place::open(app, component, Some(path.clone()));
            Ok(())
        }
        [_] => Err("Smart Edit takes a face, a feature, a plane or a part".into()),
        _ => Err("Smart Edit takes one pick".into()),
    }
}

/// Edit feature `id`: its sketch (or itself, a sketch), else its dialog; in `placement` when it
/// belongs to a part placed in the assembly.
fn feature(app: &mut SolveApp, id: u64, placement: Option<(u64, Vec<u64>)>) {
    let sketch = driving_sketch(app, id);
    let placement = placement.as_ref().map(|(c, p)| (*c, p.as_slice()));
    match sketch {
        Some(s) => app.edit_sketch_in(s, placement),
        None => app.edit_feature_in(id, placement),
    }
}

/// The sketch that drives feature `id`: itself when it is one, its profile sketch, else the first
/// sketch its definition names anywhere (a path, points to drill at, a profile in a list).
pub(crate) fn driving_sketch(app: &SolveApp, id: u64) -> Option<u64> {
    let doc = &app.session.doc;
    let f = doc.feature(id)?;
    if matches!(f.kind, FeatureKind::Sketch { .. }) {
        return Some(id);
    }
    if let Some(s) = crate::timeline::TimelineKind::of(&f.kind).profile_sketch() {
        return Some(s.0);
    }
    let is_sketch = |s: u64| doc.feature(s).is_some_and(|x| matches!(x.kind, FeatureKind::Sketch { .. }));
    fn find(v: &serde_json::Value, ok: &dyn Fn(u64) -> bool, depth: usize) -> Option<u64> {
        if depth > 8 {
            return None;
        }
        match v {
            serde_json::Value::Object(m) => {
                if let Some(s) = m.get("sketch").and_then(serde_json::Value::as_u64).filter(|s| ok(*s)) {
                    return Some(s);
                }
                m.values().find_map(|x| find(x, ok, depth + 1))
            }
            serde_json::Value::Array(a) => a.iter().find_map(|x| find(x, ok, depth + 1)),
            _ => None,
        }
    }
    let v = serde_json::to_value(&f.kind).ok()?;
    find(&v, &is_sketch, 0)
}

/// The feature that made face `index` of `body` picked at world `point`, and the placement (part,
/// path) to edit it in when the part is not the active one.
pub(crate) fn face_feature(app: &SolveApp, body: &str, index: usize, point: solidcore_record::Vec3) -> Option<(u64, Option<(u64, Vec<u64>)>)> {
    let s = &app.session;
    let component = s.world_state().ok().and_then(|st| st.body(body).map(|b| b.component))?;
    // Another part than the active one: edited where it is placed.
    let placement = (component != s.active_component && component != 0).then(|| (component, s.doc.first_path(component)));
    let path = match &placement {
        Some((_, path)) => path.clone(),
        None => s.active_occurrence_path.clone().unwrap_or_else(|| s.doc.first_path(s.active_component)),
    };
    // The pick is in the world; the history is in the part's own frame.
    let at = s
        .occurrence_placement(&path)
        .ok()
        .and_then(|placed| solidcraft_engine::doc::mat_inverse(&placed.world))
        .map_or(point, |to_model| solidcraft_engine::doc::apply_point(&to_model, point));
    let made = crate::dialogs::blend_feature(s, body, index).or_else(|| crate::dialogs::feature_of_face(s, body, index, at))?;
    Some((made, placement))
}

/// A Smart Edit carried into a linked design: the design opened in place (`part`, its file), and
/// where the pick was in that design's own coordinates; once it shows, the same pick there is
/// edited (a face, or an edge), so a part linked inside a linked assembly is reached step by step.
pub(crate) struct Follow {
    part: String,
    at: solidcore_record::Vec3,
    edge: bool,
    frames: u32,
}

/// The most frames a carried Smart Edit waits for the linked design to open.
const FOLLOW_FRAMES: u32 = 1200;

/// A pick on a body of a linked design: there is nothing to edit here (the link holds its bodies
/// only), so the design it comes from opens in place at that copy, and the pick follows. Whether
/// it did.
fn into_link(app: &mut SolveApp, body: &str, point: solidcore_record::Vec3, edge: bool) -> bool {
    let model = app.session.model.state();
    let Some((_, component, path, _)) = app.session.doc.body_frames(&model).into_iter().find(|(n, ..)| n == body) else { return false };
    if component == 0 || !app.session.doc.links.get(&component).is_some_and(|l| l.is_design()) {
        return false;
    }
    let Ok(links) = app.session.execute("component.links", &serde_json::json!({})) else { return false };
    let Some(part) = links["links"].as_array().into_iter().flatten().find(|l| l["component"].as_u64() == Some(component)).and_then(|l| l["file"].as_str()).map(str::to_string)
    else {
        return false;
    };
    // The linked design's bodies sit in the component as they sit in that design.
    let Some(at) = app.session.occurrence_placement(&path).ok().and_then(|p| solidcraft_engine::doc::mat_inverse(&p.world)).map(|inv| solidcraft_engine::doc::apply_point(&inv, point))
    else {
        return false;
    };
    app.smart_edit_follow = Some(Follow { part, at, edge, frames: 0 });
    crate::edit_in_place::open(app, component, Some(path));
    true
}

/// Each frame: a carried Smart Edit whose linked design now shows edits the same pick there.
pub(crate) fn tick(app: &mut SolveApp) {
    let Some(f) = app.smart_edit_follow.as_mut() else { return };
    f.frames += 1;
    let part = f.part.clone();
    let here = app.session.path.as_deref().is_some_and(|p| crate::documents::same_path(p, &part))
        && app.edit_in_place.active.as_ref().is_some_and(|c| crate::documents::same_path(&c.part, &part));
    if !here || app.file_load.busy() {
        if f.frames > FOLLOW_FRAMES {
            app.smart_edit_follow = None;
        }
        return;
    }
    let Some(f) = app.smart_edit_follow.take() else { return };
    match pick_at(app, f.at, f.edge) {
        Some(sel) => {
            app.session.selection = vec![sel];
            press(app);
        }
        None => app.set_status("Smart Edit: the pick is not on this design", true),
    }
}

/// The face (or edge) of the design shown nearest world point `at`, when one is close.
fn pick_at(app: &SolveApp, at: solidcore_record::Vec3, edge: bool) -> Option<Sel> {
    let world = crate::open_meshing::session_world(&app.session);
    let mut best: Option<(f64, Sel)> = None;
    for b in &world.bodies {
        let m = b.mesh();
        let tol = (m.bounds().diagonal() * 0.01).max(0.5);
        if edge {
            for (index, e) in m.edges.iter().enumerate() {
                let d = e.windows(2).map(|w| at.dist_to_segment(w[0], w[1])).fold(f64::INFINITY, f64::min);
                if d < tol && best.as_ref().is_none_or(|(bd, _)| d < *bd) {
                    best = Some((d, Sel::Edge { body: b.name.clone(), index, point: at }));
                }
            }
        } else {
            for (t, f) in m.triangles.iter().zip(&m.tri_face) {
                let Some(tri) = m.tri(t) else { continue };
                let d = crate::dialogs::point_tri_dist(at, tri);
                if d < tol && best.as_ref().is_none_or(|(bd, _)| d < *bd) {
                    best = Some((d, Sel::Face { body: b.name.clone(), index: *f as usize, point: at }));
                }
            }
        }
    }
    best.map(|(_, s)| s)
}

/// A face: the sketch of the feature that made it, else that feature, edited; on a linked design,
/// the design it comes from, opened in place, and the face there.
fn face(app: &mut SolveApp, body: &str, index: usize, point: solidcore_record::Vec3) -> Result<(), String> {
    if into_link(app, body, point, false) {
        return Ok(());
    }
    let (made, placement) = face_feature(app, body, index, point).ok_or("no feature of this part made that face")?;
    if app.session.doc.feature(made).is_some_and(|f| matches!(f.kind, FeatureKind::Import { .. } | FeatureKind::MeshImport { .. })) {
        return Err("an imported body has nothing to edit".into());
    }
    feature(app, made, placement);
    Ok(())
}

/// The faces either side of B-rep edge `index` of `body`, from the exact record.
pub(crate) fn edge_faces(app: &SolveApp, body: &str, index: usize) -> Vec<usize> {
    let Ok(st) = app.session.world_state() else { return Vec::new() };
    let Some(b) = st.body(body) else { return Vec::new() };
    let record = b.body.record();
    let mut faces: Vec<usize> = record.edges.get(index).map(|e| e.uses.iter().map(|u| u.face as usize).collect()).unwrap_or_default();
    faces.sort_unstable();
    faces.dedup();
    faces
}

/// The feature that made edge `index` of `body`: of the features that made the faces either side
/// of it, the latest in the history (a hole's edge on a top face: the hole), with the placement to
/// edit it in.
pub(crate) fn edge_feature(app: &SolveApp, body: &str, index: usize, point: solidcore_record::Vec3) -> Option<(u64, Option<(u64, Vec<u64>)>)> {
    let order = |id: u64| app.session.doc.features.iter().position(|f| f.id == id).unwrap_or(0);
    edge_faces(app, body, index)
        .into_iter()
        .filter_map(|f| face_feature(app, body, f, inside_face(app, body, f, point).unwrap_or(point)))
        .max_by_key(|(id, _)| order(*id))
}

/// A point inside face `face` of `body` near world `near`: the middle of its triangle closest to
/// it. A point on the edge itself lies on both faces and was on the model before either was made
/// (a hole's rim is on the top face too), so each face is asked about a point of its own.
fn inside_face(app: &SolveApp, body: &str, face: usize, near: solidcore_record::Vec3) -> Option<solidcore_record::Vec3> {
    let st = app.session.world_state().ok()?;
    let m = st.body(body)?.mesh();
    m.triangles
        .iter()
        .zip(&m.tri_face)
        .filter(|(_, f)| **f as usize == face)
        .filter_map(|(t, _)| m.tri(t))
        .map(|[a, b, c]| (a + b + c) * (1.0 / 3.0))
        .min_by(|x, y| x.dist(near).total_cmp(&y.dist(near)))
}

/// An edge: the feature that made it (a hole's circle: the hole), edited as a face's is, through
/// its sketch, in the part's placement when it is not the active part.
fn edge(app: &mut SolveApp, body: &str, index: usize, point: solidcore_record::Vec3) -> Result<(), String> {
    if into_link(app, body, point, true) {
        return Ok(());
    }
    let (made, placement) = edge_feature(app, body, index, point).ok_or("no feature of this part made that edge")?;
    if app.session.doc.feature(made).is_some_and(|f| matches!(f.kind, FeatureKind::Import { .. } | FeatureKind::MeshImport { .. })) {
        return Err("an imported body has nothing to edit".into());
    }
    feature(app, made, placement);
    Ok(())
}

/// Nothing picked: the sketch edited last, else the active part's newest sketch, else a new one
/// on the origin plane facing the screen.
fn nothing_picked(app: &mut SolveApp) -> Result<(), String> {
    let s = &app.session;
    let active = s.active_component;
    let mine = |id: u64| s.doc.feature(id).is_some_and(|f| f.component == active && matches!(f.kind, FeatureKind::Sketch { .. }));
    let last = app.smart_edit_last.filter(|id| mine(*id)).or_else(|| s.doc.features.iter().rev().find(|f| mine(f.id)).map(|f| f.id));
    if let Some(sketch) = last {
        app.edit_sketch(sketch);
        return Ok(());
    }
    // A new sketch, on the plane that faces the screen most.
    let back = app.cam.basis().2;
    let plane = [("XY", back.z), ("XZ", back.y), ("YZ", back.x)].into_iter().max_by(|a, b| a.1.abs().total_cmp(&b.1.abs())).map_or("XY", |p| p.0);
    app.session.selection = vec![Sel::Plane { name: plane.into() }];
    app.start("sketch.create");
    Ok(())
}

#[cfg(test)]
#[path = "smart_edit_tests.rs"]
mod tests;
