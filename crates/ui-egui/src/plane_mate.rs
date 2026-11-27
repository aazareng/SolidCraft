//! Plane Mate: one key, pressed again for each step, that ties two parts by their origin planes.
//!
//! 1. Faces, edges and whole parts become the origin plane (XY, XZ, YZ) of the part they are on,
//!    the top-level placement for a part inside a sub-assembly: the one most parallel to a flat
//!    face, or to the screen for anything else. Nothing picked: the design's own plane facing the
//!    screen. One plane picked: the design's plane most parallel to it joins it.
//! 2. Two planes on two parts: a Parallel constraint between them, the selection kept.
//! 3. Again on the same two: that Parallel becomes coincident (Flush when the planes face the same
//!    way now, Mate when they face each other, so nothing turns over), and the selection clears.
//!
//! The part that stays is the fixed one (the design, or a grounded part) when there is one, else
//! the first picked. Each step is its own undo step.

use serde_json::{Value, json};
use solidcore_record::Vec3;
use solidcraft_engine::Sel;
use solidcraft_engine::doc::joints::{Constraint, Joint, JointOrigin, Snap};
use solidcraft_engine::doc::{Document, PlaneRef};
use solidcraft_engine::geom::prelude::*;

use crate::SolveApp;
use crate::dialogs_assembly::{SnapKind, picked_snap};

/// The origin planes, in the order a tie goes to.
const PLANES: [&str; 3] = ["XY", "XZ", "YZ"];
/// Two scores this close are a tie (cosines).
const TIE: f64 = 1e-9;

/// One press of the key.
pub fn press(app: &mut SolveApp) {
    match step(app) {
        Ok(Some(done)) => app.set_status(done, false),
        Ok(None) => {}
        Err(why) => app.set_status(why, true),
    }
    // The quick constrain bar would offer the planes again: Plane Mate has them now.
    crate::quick_constrain::dismiss(app);
}

/// A picked plane: the part it is on (0 for the design) and its origin plane's name.
#[derive(Clone, Debug, PartialEq)]
struct Planed {
    part: u64,
    plane: String,
}

fn step(app: &mut SolveApp) -> Result<Option<String>, String> {
    let sels = app.session.selection.clone();
    let planes: Vec<Option<Planed>> = sels.iter().map(|s| as_plane(app, s)).collect();
    if sels.is_empty() {
        let screen = back(app);
        let name = nearest(&app.session.doc, 0, screen, screen);
        app.session.selection = vec![sel_of(&app.session.doc, &Planed { part: 0, plane: name.into() })];
        return Ok(Some(format!("Plane Mate: the assembly's {name}; pick a face on a part and press again")));
    }
    if let Some(all) = planes.iter().cloned().collect::<Option<Vec<Planed>>>() {
        match all.as_slice() {
            [one] if one.part != 0 => {
                // The design's plane most parallel to it joins it.
                let n = normal(&app.session.doc, one);
                let name = nearest(&app.session.doc, 0, n, back(app));
                app.session.selection.push(sel_of(&app.session.doc, &Planed { part: 0, plane: name.into() }));
                return Ok(Some(format!("Plane Mate: {} and the assembly's {name}; press again to make them parallel", describe(app, one))));
            }
            [_] => return Err("Plane Mate: that is the assembly's own plane; pick a face on a part as well".into()),
            [a, b] => return mate(app, a, b),
            _ => return Err("Plane Mate ties two planes; for three or four faces use the quick constrain bar".into()),
        }
    }
    // Faces, edges, parts: each becomes its part's origin plane.
    let screen = back(app);
    let mut out: Vec<Sel> = Vec::new();
    for (sel, planed) in sels.iter().zip(&planes) {
        let made = match planed {
            Some(_) => Some(sel.clone()),
            None => to_plane(app, sel, screen).map(|p| sel_of(&app.session.doc, &p)),
        };
        if let Some(made) = made
            && !out.contains(&made)
        {
            out.push(made);
        }
    }
    if out.is_empty() {
        return Err(format!("Plane Mate takes faces, edges, parts or planes (picked: {})", kinds(&sels)));
    }
    let said: Vec<String> = out.iter().filter_map(|s| as_plane(app, s)).map(|p| describe(app, &p)).collect();
    let next = match said.len() {
        1 => "press again to add the assembly's plane",
        2 => "press again to make them parallel",
        _ => "Plane Mate ties two of them",
    };
    app.session.selection = out;
    Ok(Some(format!("Plane Mate: {}; {next}", said.join(" and "))))
}

/// A plane as the status line names it: `Plate:1 XY`, or `the assembly's XY`.
fn describe(app: &SolveApp, p: &Planed) -> String {
    if p.part == 0 {
        return format!("the assembly's {}", p.plane);
    }
    let name = app.session.doc.occurrences.iter().find(|o| o.id == p.part).map_or_else(|| format!("#{}", p.part), |o| o.name.clone());
    format!("{name} {}", p.plane)
}

/// What kinds of things are picked, for a refusal.
fn kinds(sels: &[Sel]) -> String {
    let names: Vec<&str> = sels
        .iter()
        .map(|s| match s {
            Sel::Face { .. } => "face",
            Sel::Edge { .. } => "edge",
            Sel::Vertex { .. } => "point",
            Sel::Occurrence { .. } => "part",
            Sel::Body { .. } => "body",
            Sel::Plane { .. } => "plane",
            Sel::Feature { .. } => "feature",
            _ => "other",
        })
        .collect();
    names.join(", ")
}

/// The top-level placement a placement sits in (itself at the top), 0 for the design.
fn top(doc: &Document, mut occ: u64) -> u64 {
    for _ in 0..1000 {
        let Some(o) = doc.occurrences.iter().find(|o| o.id == occ) else { return 0 };
        if o.parent == 0 {
            return o.id;
        }
        let Some(up) = doc.occurrence_of(o.parent) else { return o.id };
        occ = up.id;
    }
    occ
}

/// Which way the screen faces (towards the person).
fn back(app: &SolveApp) -> Vec3 {
    app.cam.basis().2
}

/// The world normal of origin plane `name` of `part`.
fn plane_normal(doc: &Document, part: u64, name: &str) -> Vec3 {
    let local = solidcore_record::Plane::named(name).and_then(|p| p.normal().normalized()).unwrap_or(Vec3::Z);
    if part == 0 {
        return local;
    }
    let w = solidcraft_engine::doc::joints::occurrence_world(doc, part);
    solidcraft_engine::doc::apply_vector(&w, local).normalized().unwrap_or(local)
}

fn normal(doc: &Document, p: &Planed) -> Vec3 {
    plane_normal(doc, p.part, &p.plane)
}

/// The origin plane of `part` most parallel to `dir` (either way); a tie goes to the one facing
/// the screen `screen` most, then to the first of XY, XZ, YZ.
fn nearest(doc: &Document, part: u64, dir: Vec3, screen: Vec3) -> &'static str {
    let mut best = PLANES[0];
    let mut score = (f64::NEG_INFINITY, f64::NEG_INFINITY);
    for name in PLANES {
        let n = plane_normal(doc, part, name);
        let s = (n.dot(dir).abs(), n.dot(screen).abs());
        if s.0 > score.0 + TIE || ((s.0 - score.0).abs() <= TIE && s.1 > score.1 + TIE) {
            best = name;
            score = s;
        }
    }
    best
}

/// A pick that already is an origin plane of a placement or of the design.
fn as_plane(app: &SolveApp, sel: &Sel) -> Option<Planed> {
    let Sel::Plane { .. } = sel else { return None };
    let snap = picked_snap(app, sel)?;
    let plane = snap.param.get("plane")?.as_str()?;
    let name = plane.rsplit('/').next().unwrap_or(plane).to_ascii_uppercase();
    PLANES.contains(&name.as_str()).then(|| Planed { part: snap.param.get("occurrence").and_then(Value::as_u64).unwrap_or(0), plane: name })
}

/// A face, edge, vertex or part as the origin plane of its (top-level) part.
fn to_plane(app: &SolveApp, sel: &Sel, screen: Vec3) -> Option<Planed> {
    let doc = &app.session.doc;
    let (part, dir) = match sel {
        Sel::Occurrence { path } => (*path.first()?, screen),
        // A body: the part it is in.
        Sel::Body { name } => {
            let component = app.session.world_state().ok()?.body(name)?.component;
            let occ = if component == 0 { 0 } else { doc.occurrence_of(component)?.id };
            (top(doc, occ), screen)
        }
        Sel::Face { .. } | Sel::Edge { .. } | Sel::Vertex { .. } => {
            let snap = picked_snap(app, sel)?;
            let occ = snap.param.get("occurrence").and_then(Value::as_u64).unwrap_or(0);
            let flat = matches!(sel, Sel::Face { .. }) && snap.kind == SnapKind::FaceCenter && snap.param.get("face").is_some();
            (top(doc, occ), if flat { snap.z } else { screen })
        }
        _ => return None,
    };
    Some(Planed { part, plane: nearest(doc, part, dir, screen).into() })
}

/// The pick of a plane, by its placement's name where that is unique.
fn sel_of(doc: &Document, p: &Planed) -> Sel {
    if p.part == 0 {
        return Sel::Plane { name: p.plane.clone() };
    }
    let name = doc.occurrences.iter().find(|o| o.id == p.part).map(|o| o.name.clone()).unwrap_or_default();
    let unique = !name.is_empty() && doc.occurrences.iter().filter(|o| o.name == name).count() == 1;
    let prefix = if unique { name } else { p.part.to_string() };
    Sel::Plane { name: format!("{prefix}/{}", p.plane) }
}

/// The reference a constraint takes for the plane.
fn reference(p: &Planed) -> Value {
    json!({"occurrence": p.part, "plane": p.plane})
}

/// Whether a joint side is the plane `p`.
fn is(o: &JointOrigin, p: &Planed) -> bool {
    let Snap::Plane { plane, .. } = &o.snap else { return false };
    match plane.as_ref() {
        PlaneRef::Origin { name } => o.occurrence == p.part && name.eq_ignore_ascii_case(&p.plane),
        PlaneRef::Datum { datum } => {
            datum.path.last().copied().unwrap_or(0) == p.part && datum.item.name().eq_ignore_ascii_case(&p.plane)
        }
        _ => false,
    }
}

/// The constraint already between the two planes, either way round.
fn between<'a>(doc: &'a Document, a: &Planed, b: &Planed) -> Option<&'a Joint> {
    doc.assembly.joints.iter().filter(|j| j.constraint.is_some() && !j.suppressed).find(|j| (is(&j.a, a) && is(&j.b, b)) || (is(&j.a, b) && is(&j.b, a)))
}

/// A part that never moves: the design itself, or a grounded placement.
fn fixed(doc: &Document, part: u64) -> bool {
    part == 0 || doc.occurrences.iter().any(|o| o.id == part && o.grounded)
}

/// How the two planes face each other now (the cosine between their normals), from where the
/// parts are placed now, moves still being solved included.
fn facing(app: &SolveApp, a: &Planed, b: &Planed) -> f64 {
    let n = |p: &Planed| {
        let local = solidcore_record::Plane::named(&p.plane).and_then(|x| x.normal().normalized()).unwrap_or(Vec3::Z);
        if p.part == 0 {
            return local;
        }
        match app.session.occurrence_placement(&[p.part]) {
            Ok(at) => solidcraft_engine::doc::apply_vector(&at.world, local).normalized().unwrap_or(local),
            Err(_) => normal(&app.session.doc, p),
        }
    };
    n(a).dot(n(b))
}

/// Steps 2 and 3 for two planes. Each keeps the way the planes face now, so the moving part is
/// never turned over: a joint that does turn it is turned back.
fn mate(app: &mut SolveApp, a: &Planed, b: &Planed) -> Result<Option<String>, String> {
    if a.part == b.part {
        return Err("both planes are on the same part".into());
    }
    let doc = app.session.doc.clone();
    let before = facing(app, a, b);
    match between(&doc, a, b).map(|j| (j.name.clone(), j.constraint)) {
        None => {
            // The fixed part stays; else the first picked.
            let (held, moved) = if fixed(&doc, b.part) && !fixed(&doc, a.part) { (b, a) } else { (a, b) };
            let made = app.run("assembly.constraint.create", json!({"type": Constraint::Parallel.name(), "a": reference(held), "b": reference(moved)}))?;
            if before * facing(app, a, b) < 0.0
                && let Some(name) = made.get("name").and_then(Value::as_str)
            {
                app.run("assembly.constraint.edit", json!({"constraint": name, "flip": true}))?;
            }
            Ok(Some("Parallel: press Plane Mate again to make them coincident".into()))
        }
        Some((name, Some(Constraint::Parallel))) => {
            let (rule, other) = if before > 0.0 { (Constraint::Flush, Constraint::Mate) } else { (Constraint::Mate, Constraint::Flush) };
            app.run("assembly.constraint.edit", json!({"constraint": name, "type": rule.name()}))?;
            let rule = if before * facing(app, a, b) < 0.0 {
                app.run("assembly.constraint.edit", json!({"constraint": name, "type": other.name()}))?;
                other
            } else {
                rule
            };
            app.session.selection.clear();
            Ok(Some(format!("{}: the planes are coincident", crate::dialogs_constrain::label(rule))))
        }
        Some((_, Some(Constraint::Mate | Constraint::Flush))) => {
            app.session.selection.clear();
            Ok(Some("the planes are already coincident".into()))
        }
        Some((_, rule)) => Err(format!("the planes already have a {} constraint", rule.map_or("", crate::dialogs_constrain::label))),
    }
}

#[cfg(test)]
#[path = "plane_mate_tests.rs"]
mod tests;
