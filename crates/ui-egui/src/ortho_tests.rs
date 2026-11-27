//! Ortho: lines horizontal or vertical as they nearly are, arcs tangent where they join.
use serde_json::{Value, json};
use solidcraft_engine::Sel;
use solidcraft_engine::sketch::{ConstraintKind, CurveKind, Sketch};

use crate::scenario::Harness;

fn sketching() -> Harness {
    let mut h = Harness::new();
    h.app.home.open = false;
    h.app.run("sketch.create", json!({"plane": "XY"})).unwrap();
    h
}

fn draw(h: &mut Harness, id: &str, p: Value) {
    h.app.run(id, p).unwrap_or_else(|e| panic!("{id}: {e}"));
}

fn sketch(h: &Harness) -> Sketch {
    let sid = h.app.session.active_sketch.unwrap();
    h.app.session.world_state().unwrap().sketch(sid).unwrap().sketch.clone()
}

fn count(sk: &Sketch, f: impl Fn(&ConstraintKind) -> bool) -> usize {
    sk.constraints.iter().filter(|c| f(&c.kind)).count()
}

fn horizontal(k: &ConstraintKind) -> bool {
    matches!(k, ConstraintKind::Horizontal { .. })
}

fn vertical(k: &ConstraintKind) -> bool {
    matches!(k, ConstraintKind::Vertical { .. })
}

/// Nothing selected: a nearly flat line goes horizontal, a nearly upright one vertical, a 45° one
/// is left alone, all in one undo step; a second press adds nothing twice.
#[test]
fn every_line_squares_up_and_45_degrees_is_left_alone() {
    let mut h = sketching();
    draw(&mut h, "sketch.line", json!({"points": [[0, 0], [50, 3]]}));
    draw(&mut h, "sketch.line", json!({"points": [[60, 0], [63, 40]]}));
    draw(&mut h, "sketch.line", json!({"points": [[0, 10], [-20, 30]]}));
    let sk = sketch(&h);
    assert_eq!(count(&sk, horizontal) + count(&sk, vertical), 0, "drawn without any");
    let undo = h.app.session.undo.len();
    h.app.session.selection.clear();
    crate::keymap::press(&mut h.app, "Shift+O", false);
    let sk = sketch(&h);
    assert_eq!(count(&sk, horizontal), 1, "{:?}", sk.constraints);
    assert_eq!(count(&sk, vertical), 1, "{:?}", sk.constraints);
    assert_eq!(h.app.session.undo.len(), undo + 1, "one undo step");
    // The flat line is level now.
    let CurveKind::Line { a, b } = sk.curves[0].kind else { panic!("a line") };
    assert!((sk.points[a].pos.y - sk.points[b].pos.y).abs() < 1e-9);
    // Again: nothing more is added.
    h.app.start("ui.ortho");
    assert_eq!(sketch(&h).constraints.len(), sk.constraints.len());
}

/// An arc picked whose start is joined to a line's end: made tangent to it.
#[test]
fn a_picked_arc_turns_tangent_to_the_line_it_joins() {
    let mut h = sketching();
    draw(&mut h, "sketch.line", json!({"points": [[-20, 0], [0, 0]]}));
    draw(&mut h, "sketch.arc.center_point", json!({"center": [0, 12], "start": [0, 0], "end": [12, 12], "id": "arc"}));
    let line = sketch(&h).curves.iter().find(|c| c.id != "arc").unwrap().id.clone();
    let _ = h.app.run("sketch.constraint.coincident", json!({"a": format!("{line}.end"), "b": "arc.start"}));
    let tangents = |h: &Harness| count(&sketch(h), |k| matches!(k, ConstraintKind::Tangent { .. }));
    let before = tangents(&h);
    h.app.session.selection = vec![Sel::SketchCurve { id: "arc".into(), sketch: None }];
    h.app.start("ui.ortho");
    assert_eq!(tangents(&h), before + 1, "{:?}", sketch(&h).constraints);
    assert!(h.app.session.selection.is_empty());
}
