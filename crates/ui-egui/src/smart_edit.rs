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

/// A face: the sketch of the feature that made it, else that feature, edited.
fn face(app: &mut SolveApp, body: &str, index: usize, point: solidcore_record::Vec3) -> Result<(), String> {
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
