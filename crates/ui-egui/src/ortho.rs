//! Ortho: one key that squares up the open sketch, as one undo step.
//!
//! - Two sketch points (or one, and the origin): horizontal or vertical to each other, whichever
//!   they are nearer.
//! - Lines: each horizontal or vertical, whichever it is nearer. A line at 45° is left alone, as is
//!   one the sketch already holds (the constraint is refused as redundant, not added twice).
//! - Arcs: tangent to each line or arc joined at one of their ends, where not already.
//!
//! With nothing selected it does this to every line and arc of the sketch that is not
//! construction or projected geometry.

use serde_json::json;
use solidcraft_engine::Sel;
use solidcraft_engine::sketch::{ConstraintKind, CurveKind, Sketch};

use crate::SolveApp;

/// Two directions this close are a tie (a 45° line), in mm.
const TIE: f64 = 1e-9;

/// One press of the key.
pub fn press(app: &mut SolveApp) {
    match run(app) {
        Ok(done) => {
            app.session.selection.clear();
            app.set_status(done, false);
        }
        Err(why) => app.set_status(why, true),
    }
}

/// What a press did, to report.
#[derive(Default, Debug, PartialEq)]
struct Tally {
    squared: usize,
    tangent: usize,
    held: usize,
    diagonal: usize,
}

impl Tally {
    fn report(&self) -> String {
        let mut parts = vec![format!("{} horizontal/vertical", self.squared)];
        if self.tangent > 0 {
            parts.push(format!("{} tangent", self.tangent));
        }
        if self.held > 0 {
            parts.push(format!("{} already held", self.held));
        }
        if self.diagonal > 0 {
            parts.push(format!("{} at 45° left alone", self.diagonal));
        }
        format!("Ortho: {}", parts.join(", "))
    }
}

fn run(app: &mut SolveApp) -> Result<String, String> {
    let sid = app.session.active_sketch.ok_or("Ortho works in an open sketch")?;
    let sk = app.session.world_state().map_err(|e| e.to_string())?.sketch(sid).map(|s| s.sketch.clone()).ok_or("the sketch is not there")?;
    let sel = app.session.selection.clone();
    let mut tally = Tally::default();

    // Points: one or two, and nothing else.
    let points: Vec<String> = sel.iter().filter_map(|s| if let Sel::SketchPoint { id } = s { Some(id.clone()) } else { None }).collect();
    if !points.is_empty() && points.len() == sel.len() {
        let (p, q) = match points.as_slice() {
            [p] => (p.clone(), "origin".to_string()),
            [p, q] => (p.clone(), q.clone()),
            _ => return Err("Ortho takes one or two points, or lines and arcs".into()),
        };
        let mut params = json!({"points": [p, q], "if_redundant": "refuse"});
        if let (Some(a), Some(b)) = (position(&sk, &p), position(&sk, &q)) {
            match square(b.0 - a.0, b.1 - a.1) {
                Some(mode) => params["mode"] = json!(mode),
                None => return Err("the points are at 45° to each other: Ortho leaves them alone".into()),
            }
        }
        app.run("sketch.constraint.horizontal_vertical", params).map_err(|e| format!("Ortho: {e}"))?;
        tally.squared = 1;
        return Ok(tally.report());
    }

    // Curves: those picked, or every drawn one.
    let picked: Vec<usize> = sel
        .iter()
        .filter_map(|s| if let Sel::SketchCurve { id, .. } = s { sk.curves.iter().position(|c| c.id == *id) } else { None })
        .collect();
    let curves: Vec<usize> = if sel.is_empty() {
        (0..sk.curves.len()).filter(|i| sk.curves.get(*i).is_some_and(|c| !c.construction && !c.centerline && c.link.is_none())).collect()
    } else {
        picked
    };
    if curves.is_empty() {
        return Err("Ortho takes lines, arcs or one or two points".into());
    }
    crate::quick_constrain::one_step(app, "Ortho", |app| {
        for &i in &curves {
            let Some(c) = sk.curves.get(i) else { continue };
            match c.kind {
                CurveKind::Line { a, b } => {
                    if holds_angle(&sk, i) {
                        tally.held += 1;
                        continue;
                    }
                    let (Some(pa), Some(pb)) = (sk.points.get(a), sk.points.get(b)) else { continue };
                    let Some(mode) = square(pb.pos.x - pa.pos.x, pb.pos.y - pa.pos.y) else {
                        tally.diagonal += 1;
                        continue;
                    };
                    match app.run("sketch.constraint.horizontal_vertical", json!({"line": c.id, "mode": mode, "if_redundant": "refuse"})) {
                        Ok(_) => tally.squared += 1,
                        Err(_) => tally.held += 1,
                    }
                }
                CurveKind::Arc { a, b, .. } => {
                    for j in joined(&sk, i, [a, b]) {
                        let Some(other) = sk.curves.get(j) else { continue };
                        if tangent(&sk, i, j) {
                            tally.held += 1;
                            continue;
                        }
                        match app.run("sketch.constraint.tangent", json!({"a": c.id, "b": other.id})) {
                            Ok(_) => tally.tangent += 1,
                            Err(_) => tally.held += 1,
                        }
                    }
                }
                _ => {}
            }
        }
        if tally.squared + tally.tangent == 0 { Err(tally.report()) } else { Ok(()) }
    })?;
    Ok(tally.report())
}

/// Horizontal or vertical, whichever the run `(dx, dy)` is nearer; `None` at 45° or for no run.
fn square(dx: f64, dy: f64) -> Option<&'static str> {
    let (x, y) = (dx.abs(), dy.abs());
    if !x.is_finite() || !y.is_finite() || (x - y).abs() <= TIE {
        return None;
    }
    Some(if x > y { "horizontal" } else { "vertical" })
}

/// Where a point id lies in the sketch; the origin at (0, 0).
fn position(sk: &Sketch, id: &str) -> Option<(f64, f64)> {
    if id == "origin" {
        return Some((0.0, 0.0));
    }
    sk.points.iter().find(|p| p.id == id).map(|p| (p.pos.x, p.pos.y))
}

/// The sketch already holds line `l`'s direction on its own: horizontal, vertical, or an angle.
fn holds_angle(sk: &Sketch, l: usize) -> bool {
    sk.constraints.iter().any(|c| match &c.kind {
        ConstraintKind::Horizontal { l: x } | ConstraintKind::Vertical { l: x } => *x == l,
        _ => false,
    })
}

/// Curves `a` and `b` are already tangent.
fn tangent(sk: &Sketch, a: usize, b: usize) -> bool {
    sk.constraints.iter().any(|c| matches!(c.kind, ConstraintKind::Tangent { a: x, b: y } if (x == a && y == b) || (x == b && y == a)))
}

/// The ends of a line or an arc.
fn ends(kind: &CurveKind) -> Option<[usize; 2]> {
    match *kind {
        CurveKind::Line { a, b } | CurveKind::Arc { a, b, .. } => Some([a, b]),
        _ => None,
    }
}

/// The points that are one with `p`: itself and those coincident with it.
fn same(sk: &Sketch, p: usize) -> Vec<usize> {
    let mut out = vec![p];
    for c in &sk.constraints {
        if let ConstraintKind::Coincident { p: x, q: y } = c.kind {
            if x == p {
                out.push(y);
            } else if y == p {
                out.push(x);
            }
        }
    }
    out
}

/// The drawn lines and arcs joined to curve `i` at one of `ends`.
fn joined(sk: &Sketch, i: usize, own: [usize; 2]) -> Vec<usize> {
    let mut out = Vec::new();
    for end in own {
        let here = same(sk, end);
        for (j, c) in sk.curves.iter().enumerate() {
            if j == i || c.construction || c.centerline || out.contains(&j) {
                continue;
            }
            if ends(&c.kind).is_some_and(|e| e.iter().any(|p| here.contains(p))) {
                out.push(j);
            }
        }
    }
    out
}

#[cfg(test)]
#[path = "ortho_tests.rs"]
mod tests;
