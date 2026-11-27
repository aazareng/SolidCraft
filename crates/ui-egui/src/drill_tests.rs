//! Drill Up and Drill Down: up to what holds the last pick, and back down.
use serde_json::{Value, json};
use solidcore_record::Vec3;
use solidcraft_engine::doc::FeatureKind;
use solidcraft_engine::{Sel, Session};

use crate::scenario::Harness;
use crate::{Services, SolveApp};

/// A pick of the face a point of the exact geometry lies on.
fn face(app: &SolveApp, p: Vec3) -> Sel {
    let st = app.session.world_state().unwrap();
    let mut best: Option<(f64, Sel)> = None;
    for b in &st.bodies {
        let m = b.mesh();
        for (t, f) in m.triangles.iter().zip(&m.tri_face) {
            let Some([a, bb, c]) = m.tri(t) else { continue };
            let d = crate::preview::point_triangle_dist(p, a, bb, c);
            if best.as_ref().is_none_or(|(bd, _)| d < *bd) {
                best = Some((d, Sel::Face { body: b.name.clone(), index: *f as usize, point: p }));
            }
        }
    }
    best.map(|x| x.1).unwrap()
}

/// A face of a placed part goes up to the part; Drill Down brings the face back. The other pick
/// stays.
#[test]
fn a_face_goes_up_to_its_part_and_back_down() {
    let mut s = Session::default();
    let mut run = |id: &str, p: Value| s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"));
    for (name, corner) in [("A", [0, 0, 0]), ("B", [40, 0, 0])] {
        run("component.activate", json!({"component": "root"}));
        run("component.create", json!({ "name": name }));
        run("solid.box", json!({"length": 20, "width": 20, "height": 20, "corner": corner}));
    }
    run("component.activate", json!({"component": "root"}));
    drop(run);
    let b = s.doc.occurrences.iter().find(|o| o.name == "B:1").unwrap().id;
    let mut h = Harness::new();
    *h.app = SolveApp::new(s, Services::default());
    h.app.home.open = false;
    h.frames(3);
    let other = face(&h.app, Vec3::new(10.0, 10.0, 20.0));
    let top = face(&h.app, Vec3::new(50.0, 10.0, 20.0));
    h.app.session.selection = vec![other.clone(), top.clone()];
    crate::keymap::press(&mut h.app, "Alt+Up", false);
    assert_eq!(h.app.session.selection, vec![other.clone(), Sel::Occurrence { path: vec![b] }]);
    h.app.start("ui.drill_up");
    assert_eq!(h.app.session.selection.len(), 2, "a top-level part has nothing above it");
    h.app.start("ui.drill_down");
    assert_eq!(h.app.session.selection, vec![other, top]);
}

/// Alt + wheel over the view drills: a notch up takes the face to its part, a notch down brings
/// it back, and the view doesn't zoom.
#[test]
fn alt_and_the_wheel_drill_up_and_down_without_zooming() {
    let mut s = Session::default();
    let mut run = |id: &str, p: Value| s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"));
    for (name, corner) in [("A", [0, 0, 0]), ("B", [40, 0, 0])] {
        run("component.activate", json!({"component": "root"}));
        run("component.create", json!({ "name": name }));
        run("solid.box", json!({"length": 20, "width": 20, "height": 20, "corner": corner}));
    }
    run("component.activate", json!({"component": "root"}));
    drop(run);
    let b = s.doc.occurrences.iter().find(|o| o.name == "B:1").unwrap().id;
    let mut h = Harness::new();
    *h.app = SolveApp::new(s, Services::default());
    h.app.home.open = false;
    h.frames(3);
    let c = h.app.viewport.rect.expect("a view").center();
    h.step(&json!({"move": [c.x, c.y]})).unwrap();
    let top = face(&h.app, Vec3::new(50.0, 10.0, 20.0));
    h.app.session.selection = vec![top.clone()];
    // Let the view settle (smooth zoom and fitting run over several frames).
    h.frames(30);
    let distance = h.app.cam.distance;
    let notch = |h: &mut Harness, dy: f32, modifiers: egui::Modifiers| {
        h.app.synthetic.push(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: egui::vec2(0.0, dy),
            modifiers,
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Unknown,
        });
        h.frames(30);
    };
    notch(&mut h, 1.0, egui::Modifiers::ALT);
    assert_eq!(h.app.session.selection, vec![Sel::Occurrence { path: vec![b] }]);
    assert_eq!(h.app.cam.distance, distance, "Alt + wheel up didn't zoom");
    notch(&mut h, -1.0, egui::Modifiers::ALT);
    assert_eq!(h.app.session.selection, vec![top.clone()]);
    assert_eq!(h.app.cam.distance, distance, "Alt + wheel down didn't zoom");
    // Without Alt the wheel zooms as before and the pick stays.
    notch(&mut h, 1.0, egui::Modifiers::NONE);
    assert_ne!(h.app.cam.distance, distance, "the plain wheel zooms");
    assert_eq!(h.app.session.selection, vec![top]);
}

/// A feature goes up to the one before it and down to the one after; in a sketch a line goes up
/// to its rectangle and down to its midpoint.
#[test]
fn features_step_through_history_and_sketch_lines_chain_and_split() {
    let mut h = Harness::new();
    h.app.home.open = false;
    for (id, p) in [
        ("sketch.create", json!({"plane": "XY"})),
        ("sketch.rectangle.two_point", json!({"p0": [0, 0], "p1": [60, 20]})),
        ("sketch.finish", json!({})),
        ("solid.extrude", json!({"distance": 10})),
    ] {
        h.app.run(id, p).unwrap_or_else(|e| panic!("{id}: {e}"));
    }
    h.frames(2);
    let sketch = h.app.session.doc.features.iter().find(|f| matches!(f.kind, FeatureKind::Sketch { .. })).unwrap().id;
    let extrude = h.app.session.doc.features.iter().find(|f| matches!(f.kind, FeatureKind::Extrude { .. })).unwrap().id;
    h.app.session.selection = vec![Sel::Feature { id: extrude }];
    h.app.start("ui.drill_up");
    assert_eq!(h.app.session.selection, vec![Sel::Feature { id: sketch }]);
    h.app.session.selection = vec![Sel::Feature { id: sketch }];
    h.app.start("ui.drill_down");
    assert_eq!(h.app.session.selection, vec![Sel::Feature { id: extrude }]);

    h.app.edit_sketch(sketch);
    h.frames(5);
    let sid = h.app.session.active_sketch.expect("the sketch is open");
    let sk = h.app.session.world_state().unwrap().sketch(sid).unwrap().sketch.clone();
    let line = sk.curves[0].id.clone();
    h.app.session.selection = vec![Sel::SketchCurve { id: line.clone(), sketch: None }];
    h.app.start("ui.drill_up");
    assert_eq!(h.app.session.selection.len(), 4, "the whole rectangle: {:?}", h.app.session.selection);
    h.app.session.selection = vec![Sel::SketchCurve { id: line.clone(), sketch: None }];
    h.app.start("ui.drill_down");
    assert_eq!(h.app.session.selection, vec![Sel::SketchMidpoint { id: line }]);
}
