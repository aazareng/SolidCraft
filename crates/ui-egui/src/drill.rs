//! Drill Up and Drill Down: two keys that move the last pick up to what holds it, and back down.
//! The other picks stay.
//!
//! Up:
//! - A face, edge or point of a placed part: that part; a part inside a sub-assembly: the
//!   sub-assembly; the top: nothing.
//! - An edge of the design's own bodies: the edges that continue it smoothly (its tangent chain).
//! - A feature: the feature before it in its part's history.
//! - In a sketch, a line or arc: the chain of curves joined end to end with it.
//!
//! Down: back the way Drill Up came, one step a press. Otherwise:
//! - A feature: the one after it; a face of the design's own bodies: the sketch of the feature
//!   that made it.
//! - In a sketch, a line: its midpoint; an arc or circle: its centre.
//!
//! Alt + the mouse wheel over the view does the same: a notch up drills up, a notch down drills
//! down, and the wheel doesn't zoom while Alt is held (unless Preferences make Alt the wheel's zoom
//! key: then it zooms, as chosen).

use serde_json::Value;
use solidcraft_engine::Sel;
use solidcraft_engine::geom::prelude::*;
use solidcraft_engine::doc::FeatureKind;
use solidcraft_engine::sketch::{ConstraintKind, CurveKind, Sketch};

use crate::SolveApp;
use crate::dialogs_assembly::picked_snap;

/// The steps Drill Up made (what it replaced, and with what), the last one last.
#[derive(Default)]
pub struct Trail {
    steps: Vec<(Sel, Sel)>,
    /// Alt + wheel turned so far toward the next step, in notches (a trackpad scrolls in fractions).
    wheel: f32,
    /// An Alt + wheel scroll is still running out: egui smooths each notch over a few frames, and
    /// that tail must not zoom either, even once Alt is let go.
    quiet: bool,
}

/// The most steps kept.
const KEPT: usize = 64;

/// Points of a smooth (trackpad) scroll that make one notch.
const POINTS_PER_NOTCH: f32 = 50.0;

/// Each frame, before the camera reads the wheel: Alt + wheel over `rect` drills, one step a notch
/// (up: Drill Up), and those wheel events are taken so the view doesn't zoom.
pub fn wheel(app: &mut SolveApp, ui: &egui::Ui, rect: egui::Rect) {
    // The smoothed tail of an Alt + wheel scroll: swallowed until the scroll is over.
    if app.drill.quiet {
        if ui.input(|i| i.is_scrolling()) {
            ui.ctx().input_mut(|i| i.smooth_scroll_delta = egui::Vec2::ZERO);
        } else {
            app.drill.quiet = false;
        }
    }
    let over = ui.input(|i| i.pointer.hover_pos()).is_some_and(|p| rect.contains(p) && !crate::shell::covered(app, p));
    if !over {
        app.drill.wheel = 0.0;
        return;
    }
    let mut notches = 0.0f32;
    let mut taken = false;
    // Preferences may make Alt the key that zooms with the wheel: then it keeps zooming.
    let zoom_modifier = app.preferences.trackpad_zoom_modifier.modifiers();
    ui.ctx().input_mut(|i| {
        let alt_only =
            |m: &egui::Modifiers| m.alt && !m.ctrl && !m.shift && !m.command && !crate::navigation::claims_scroll(*m, zoom_modifier);
        i.events.retain(|e| match e {
            egui::Event::MouseWheel { unit, delta, modifiers, .. } if alt_only(modifiers) => {
                notches += match unit {
                    egui::MouseWheelUnit::Point => delta.y / POINTS_PER_NOTCH,
                    egui::MouseWheelUnit::Line | egui::MouseWheelUnit::Page => delta.y.signum() * delta.y.abs().max(1.0),
                };
                taken = true;
                false
            }
            _ => true,
        });
        if taken || alt_only(&i.modifiers) {
            i.smooth_scroll_delta = egui::Vec2::ZERO;
        }
    });
    if taken {
        app.drill.quiet = true;
    }
    if !notches.is_finite() || notches == 0.0 {
        return;
    }
    // A flung wheel drills a few steps at most.
    app.drill.wheel = (app.drill.wheel + notches).clamp(-4.0, 4.0);
    while app.drill.wheel >= 1.0 {
        app.drill.wheel -= 1.0;
        up(app);
    }
    while app.drill.wheel <= -1.0 {
        app.drill.wheel += 1.0;
        down(app);
    }
}

/// One press of Drill Up.
pub fn up(app: &mut SolveApp) {
    let Some(last) = app.session.selection.last().cloned() else {
        app.set_status("Drill Up: pick something first", true);
        return;
    };
    match parent(app, &last) {
        Some(Up::One(p)) => {
            replace_last(app, vec![p.clone()]);
            let trail = &mut app.drill.steps;
            trail.push((last, p));
            if trail.len() > KEPT {
                trail.remove(0);
            }
        }
        Some(Up::Many(more)) => replace_last(app, more),
        None => app.set_status("Drill Up: nothing holds it", false),
    }
}

/// One press of Drill Down.
pub fn down(app: &mut SolveApp) {
    let last = app.session.selection.last().cloned();
    // Back the way Drill Up came.
    if let Some(l) = &last
        && app.drill.steps.last().is_some_and(|(_, p)| p == l)
        && let Some((child, _)) = app.drill.steps.pop()
    {
        replace_last(app, vec![child]);
        return;
    }
    let Some(last) = last else {
        app.set_status("Drill Down: pick something first", true);
        return;
    };
    match child(app, &last) {
        Some(c) => replace_last(app, vec![c]),
        None => app.set_status("Drill Down: nothing inside it", false),
    }
}

/// What Drill Up gives: one thing in place of the pick, or the pick and more.
enum Up {
    One(Sel),
    Many(Vec<Sel>),
}

/// The selection with its last pick replaced by `with` (each once).
fn replace_last(app: &mut SolveApp, with: Vec<Sel>) {
    let sel = &mut app.session.selection;
    sel.pop();
    for s in with {
        if !sel.contains(&s) {
            sel.push(s);
        }
    }
}

/// Whether `body` is one part of a linked design that holds several: a linked assembly, whose
/// parts come in as bodies of one component (named after the part, `1 (1:4)`).
pub(crate) fn linked_part_body(app: &SolveApp, body: &str) -> bool {
    let model = app.session.model.state();
    let frames = app.session.doc.body_frames(&model);
    let Some((_, component, path, _)) = frames.iter().find(|(n, ..)| n == body) else { return false };
    *component != 0
        && app.session.doc.links.get(component).is_some_and(|l| l.is_design())
        && frames.iter().filter(|(_, c, p, _)| c == component && p == path).count() > 1
}

/// The placement path of the part a face, edge or point is on, all the way down (a part inside a
/// sub-assembly: the sub-assembly, then the part), so Drill Up takes the part first and its
/// sub-assembly next. Empty: the design's own body.
pub(crate) fn part_path(app: &SolveApp, sel: &Sel) -> Vec<u64> {
    let (Sel::Face { body, .. } | Sel::Edge { body, .. } | Sel::Vertex { body, .. }) = sel else { return Vec::new() };
    let model = app.session.model.state();
    if let Some(path) = app.session.doc.body_frames(&model).into_iter().find(|(name, ..)| name == body).map(|(_, _, path, _)| path.to_vec()) {
        return path;
    }
    // Not placed by a frame: the placement the joints would name.
    let doc = &app.session.doc;
    let occ = picked_snap(app, sel).and_then(|s| s.param.get("occurrence").and_then(Value::as_u64)).unwrap_or(0);
    let Some(o) = doc.occurrences.iter().find(|o| o.id == occ) else { return Vec::new() };
    let mut path = doc.first_path(o.parent);
    path.push(o.id);
    path
}

fn parent(app: &SolveApp, sel: &Sel) -> Option<Up> {
    let doc = &app.session.doc;
    match sel {
        Sel::Face { body, .. } | Sel::Edge { body, .. } | Sel::Vertex { body, .. } => {
            // A linked assembly comes in as one component, each of its parts a body: the part's
            // body is the step between the face and the whole linked copy.
            if linked_part_body(app, body) {
                return Some(Up::One(Sel::Body { name: body.clone() }));
            }
            let path = part_path(app, sel);
            if !path.is_empty() {
                return Some(Up::One(Sel::Occurrence { path }));
            }
            let Sel::Edge { .. } = sel else { return None };
            let chain = tangent_chain(app, sel);
            (chain.len() > 1).then_some(Up::Many(chain))
        }
        Sel::Occurrence { path } => (path.len() > 1).then(|| Up::One(Sel::Occurrence { path: path[..path.len() - 1].to_vec() })),
        // A body: the placement it sits in.
        Sel::Body { name } => {
            let model = app.session.model.state();
            let path = app.session.doc.body_frames(&model).into_iter().find(|(n, ..)| n == name).map(|(_, _, path, _)| path)?;
            (!path.is_empty()).then(|| Up::One(Sel::Occurrence { path }))
        }
        Sel::Feature { id } => {
            let f = doc.feature(*id)?;
            let at = doc.features.iter().position(|x| x.id == *id)?;
            doc.features[..at].iter().rev().find(|x| x.component == f.component).map(|x| Up::One(Sel::Feature { id: x.id }))
        }
        Sel::SketchCurve { id, .. } => {
            let sk = open_sketch(app)?;
            let chain = sketch_chain(&sk, id);
            (chain.len() > 1).then(|| Up::Many(chain.into_iter().map(|id| Sel::SketchCurve { id, sketch: None }).collect()))
        }
        _ => None,
    }
}

fn child(app: &SolveApp, sel: &Sel) -> Option<Sel> {
    let doc = &app.session.doc;
    match sel {
        Sel::Feature { id } => {
            let f = doc.feature(*id)?;
            let at = doc.features.iter().position(|x| x.id == *id)?;
            doc.features.get(at + 1..)?.iter().find(|x| x.component == f.component).map(|x| Sel::Feature { id: x.id })
        }
        Sel::Face { body, index, point } if part_path(app, sel).is_empty() => {
            let made = crate::dialogs::blend_feature(&app.session, body, *index).or_else(|| crate::dialogs::feature_of_face(&app.session, body, *index, *point))?;
            let f = doc.feature(made)?;
            let sketch = if matches!(f.kind, FeatureKind::Sketch { .. }) { Some(made) } else { crate::timeline::TimelineKind::of(&f.kind).profile_sketch().map(|s| s.0) };
            sketch.map(|id| Sel::Feature { id })
        }
        Sel::SketchCurve { id, .. } => {
            let sk = open_sketch(app)?;
            let c = sk.curves.iter().find(|c| c.id == *id)?;
            match c.kind {
                CurveKind::Line { .. } => Some(Sel::SketchMidpoint { id: id.clone() }),
                CurveKind::Arc { c, .. } | CurveKind::Circle { c, .. } => sk.points.get(c).map(|p| Sel::SketchPoint { id: p.id.clone() }),
                _ => None,
            }
        }
        _ => None,
    }
}

/// The sketch open now.
fn open_sketch(app: &SolveApp) -> Option<Sketch> {
    let sid = app.session.active_sketch?;
    app.session.world_state().ok()?.sketch(sid).map(|s| s.sketch.clone())
}

/// The ends of a line or an arc.
fn ends(kind: &CurveKind) -> Option<[usize; 2]> {
    match *kind {
        CurveKind::Line { a, b } | CurveKind::Arc { a, b, .. } => Some([a, b]),
        _ => None,
    }
}

/// The curves joined end to end with curve `id` (it first), through shared or coincident ends.
fn sketch_chain(sk: &Sketch, id: &str) -> Vec<String> {
    let Some(start) = sk.curves.iter().position(|c| c.id == id) else { return Vec::new() };
    // Points that are one: shared indices, or tied by a coincident constraint.
    let mut root: Vec<usize> = (0..sk.points.len()).collect();
    fn find(root: &mut [usize], mut p: usize) -> usize {
        while root.get(p).is_some_and(|r| *r != p) {
            p = root[p];
        }
        p
    }
    for c in &sk.constraints {
        if let ConstraintKind::Coincident { p, q } = c.kind
            && p < root.len()
            && q < root.len()
        {
            let (a, b) = (find(&mut root, p), find(&mut root, q));
            root[a] = b;
        }
    }
    let mut done = vec![start];
    let mut i = 0;
    while let Some(&cur) = done.get(i) {
        i += 1;
        let Some(e) = sk.curves.get(cur).and_then(|c| ends(&c.kind)) else { continue };
        let here: Vec<usize> = e.iter().map(|p| find(&mut root, *p)).collect();
        for (j, c) in sk.curves.iter().enumerate() {
            if done.contains(&j) || c.construction != sk.curves[start].construction {
                continue;
            }
            if ends(&c.kind).is_some_and(|f| f.iter().any(|p| here.contains(&find(&mut root, *p)))) {
                done.push(j);
            }
        }
    }
    done.into_iter().filter_map(|j| sk.curves.get(j).map(|c| c.id.clone())).collect()
}

/// The edges that continue `edge` smoothly, itself first.
fn tangent_chain(app: &SolveApp, edge: &Sel) -> Vec<Sel> {
    let Sel::Edge { body, index, .. } = edge else { return Vec::new() };
    let st = crate::open_meshing::session_world(&app.session);
    let Some(b) = st.body(body) else { return Vec::new() };
    let m = b.mesh();
    let mut out = vec![edge.clone()];
    for i in m.tangent_chain(*index, solidcraft_engine::geom::TANGENT_CHAIN) {
        if i == *index {
            continue;
        }
        let Some(mid) = m.edges.get(i).and_then(|e| e.get(e.len() / 2).copied()) else { continue };
        out.push(Sel::Edge { body: body.clone(), index: i, point: mid });
    }
    out
}

#[cfg(test)]
#[path = "drill_tests.rs"]
mod tests;
