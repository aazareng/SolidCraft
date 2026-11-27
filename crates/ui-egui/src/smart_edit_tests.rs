//! Smart Edit: one key edits the picked thing, or finishes the sketch open.
use serde_json::json;
use solidcore_record::Vec3;
use solidcraft_engine::Sel;
use solidcraft_engine::doc::FeatureKind;
use solidcraft_engine::geom::prelude::*;

use crate::scenario::Harness;

/// A 60 × 20 × 10 block extruded from a sketch named Base.
fn block() -> (Harness, u64) {
    let mut h = Harness::new();
    h.app.home.open = false;
    for (id, p) in [
        ("sketch.create", json!({"plane": "XY", "name": "Base"})),
        ("sketch.rectangle.two_point", json!({"p0": [0, 0], "p1": [60, 20]})),
        ("sketch.finish", json!({})),
        ("solid.extrude", json!({"distance": 10})),
    ] {
        h.app.run(id, p).unwrap_or_else(|e| panic!("{id}: {e}"));
    }
    h.frames(3);
    let base = h.app.session.doc.features.iter().find(|f| matches!(f.kind, FeatureKind::Sketch { .. })).unwrap().id;
    (h, base)
}

/// A pick of the face a point of the exact geometry lies on.
fn face(h: &Harness, p: Vec3) -> Sel {
    let st = h.app.session.world_state().unwrap();
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

fn press(h: &mut Harness) {
    crate::keymap::press(&mut h.app, "Shift+D", false);
    h.frames(5);
}

/// The block's top face picked: its extrude's sketch opens; pressed again, the sketch finishes.
#[test]
fn a_face_opens_its_features_sketch_and_again_finishes_it() {
    let (mut h, base) = block();
    h.app.session.selection = vec![face(&h, Vec3::new(30.0, 10.0, 10.0))];
    press(&mut h);
    assert_eq!(h.app.session.active_sketch, Some(base));
    press(&mut h);
    assert_eq!(h.app.session.active_sketch, None);
}

/// A hole's round edge picked (where it meets the block's top): the hole made it, not the extrude,
/// so the hole's points sketch opens.
#[test]
fn a_holes_edge_opens_the_holes_sketch() {
    let (mut h, _) = block();
    for (id, p) in [
        ("sketch.create", json!({"plane": {"origin": [0, 0, 10], "x_dir": [1, 0, 0], "y_dir": [0, 1, 0]}, "name": "Pts"})),
        ("sketch.point", json!({"point": [15, 10], "id": "a"})),
        ("sketch.finish", json!({})),
        ("solid.hole", json!({"sketch": "Pts", "points": ["a"], "diameter": 8, "name": "Bore"})),
    ] {
        h.app.run(id, p).unwrap_or_else(|e| panic!("{id}: {e}"));
    }
    h.frames(3);
    let pts = h.app.session.doc.features.iter().find(|f| f.name == "Pts").unwrap().id;
    // The rim's point at +x of the hole's centre (15, 10) on the top.
    let p = Vec3::new(19.0, 10.0, 10.0);
    let st = h.app.session.world_state().unwrap();
    let b = &st.bodies[0];
    let m = b.mesh();
    let index = m.edges.iter().position(|e| e.windows(2).any(|w| p.dist_to_segment(w[0], w[1]) < 0.05)).expect("the rim");
    let sel = Sel::Edge { body: b.name.clone(), index, point: p };
    drop(st);
    h.app.session.selection = vec![sel];
    press(&mut h);
    assert_eq!(h.app.session.active_sketch, Some(pts), "the hole's sketch, not the block's");
}

/// A face of a linked part (its design in another file): the part opens in place at that copy,
/// and the sketch of the feature that made the face opens there.
#[test]
fn a_linked_parts_face_opens_the_part_in_place_and_its_sketch() {
    let folder = std::env::temp_dir().join(format!("solidcraft-smart-edit-link-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    let part = folder.join("part.sc");
    let mut s = solidcraft_engine::Session::default();
    for (id, p) in [
        ("sketch.create", json!({"plane": "XY", "name": "Base"})),
        ("sketch.rectangle.two_point", json!({"p0": [0, 0], "p1": [60, 20]})),
        ("sketch.finish", json!({})),
        ("solid.extrude", json!({"distance": 10})),
        ("file.save", json!({"path": part, "overwrite": true})),
    ] {
        s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"));
    }
    let mut app = crate::SolveApp::new(solidcraft_engine::Session::default(), Default::default());
    app.run("component.insert_design", json!({"path": part, "translate": [100, 0, 0]})).unwrap();
    app.run("file.save", json!({"path": folder.join("asm.sc"), "overwrite": true})).unwrap();
    // The linked part's top, at x 100..160.
    let p = Vec3::new(130.0, 10.0, 10.0);
    let st = app.session.world_state().unwrap();
    let b = &st.bodies[0];
    let m = b.mesh();
    let (_, f) = m.triangles.iter().zip(&m.tri_face).find(|(t, _)| m.tri(t).is_some_and(|tri| crate::preview::point_triangle_dist(p, tri[0], tri[1], tri[2]) < 1e-6)).expect("the top");
    let sel = Sel::Face { body: b.name.clone(), index: *f as usize, point: p };
    drop(st);
    app.session.selection = vec![sel];
    crate::smart_edit::press(&mut app);
    let ctx = egui::Context::default();
    let began = std::time::Instant::now();
    while app.session.active_sketch.is_none() {
        crate::file_load::tick(&mut app, &ctx);
        crate::edit_in_place::tick(&mut app);
        crate::smart_edit::tick(&mut app);
        assert!(began.elapsed() < std::time::Duration::from_secs(120), "the part's sketch never opened");
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(app.session.path.as_deref().is_some_and(|p| crate::documents::same_path(p, &part.to_string_lossy())), "the part is shown");
    assert!(app.edit_in_place.active.is_some(), "in place, the assembly around it");
    let base = app.session.doc.features.iter().find(|f| f.name == "Base").unwrap().id;
    assert_eq!(app.session.active_sketch, Some(base));
    let _ = std::fs::remove_dir_all(&folder);
}

/// Nothing picked: the sketch edited last opens; in an empty design, a new sketch starts on the
/// plane facing the screen.
#[test]
fn nothing_picked_opens_the_last_sketch_or_starts_one() {
    let (mut h, base) = block();
    h.app.session.selection.clear();
    press(&mut h);
    assert_eq!(h.app.session.active_sketch, Some(base));

    let mut h = Harness::new();
    h.app.home.open = false;
    h.frames(2);
    press(&mut h);
    let started = h.app.session.active_sketch.is_some() || h.app.dialog.as_ref().is_some_and(|d| matches!(d.kind, crate::dialogs::Kind::Sketch));
    assert!(started, "a new sketch starts");
}
