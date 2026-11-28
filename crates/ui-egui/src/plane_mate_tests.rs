//! Plane Mate: picks become origin planes, then parallel, then coincident, one press each.
use serde_json::{Value, json};
use solidcore_record::Vec3;
use solidcraft_engine::doc::joints::Constraint;
use solidcraft_engine::{Sel, Session};

use crate::scenario::Harness;
use crate::{Services, SolveApp};

/// A grounded 100 mm plate (top at z = 10) and a free 20 mm block, turned 10° about the X line
/// through its middle.
fn plate_and_turned_block() -> (Harness, u64) {
    let mut s = Session::default();
    let mut run = |id: &str, p: Value| s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"));
    for (name, size, corner) in [("Plate", [100, 100, 10], [0, 0, 0]), ("Block", [20, 20, 20], [30, 40, 50])] {
        run("component.activate", json!({"component": "root"}));
        run("component.create", json!({ "name": name }));
        run("solid.box", json!({"length": size[0], "width": size[1], "height": size[2], "corner": corner}));
    }
    run("component.activate", json!({"component": "root"}));
    run("occurrence.ground", json!({"occurrence": "Plate:1", "grounded": true}));
    drop(run);
    let block = s.doc.occurrences.iter().find(|o| o.name == "Block:1").unwrap().id;
    let turn = json!({"origin": [40, 50, 60], "axis": [1, 0, 0], "angle": 10f64.to_radians()});
    s.execute("occurrence.drag", &json!({"occurrence": block, "rotate": turn})).unwrap();
    let mut h = Harness::new();
    *h.app = SolveApp::new(s, Services::default());
    h.app.home.open = false;
    h.frames(3);
    (h, block)
}

/// Look straight down: the screen faces +Z.
fn look_down(app: &mut SolveApp) {
    app.cam.yaw = 0.0;
    app.cam.pitch = std::f64::consts::FRAC_PI_2 - 1e-3;
    app.cam.roll = 0.0;
}

fn plane(name: &str) -> Sel {
    Sel::Plane { name: name.into() }
}

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

fn block_world(h: &Harness, block: u64) -> solidcraft_engine::doc::Mat {
    h.app.session.occurrence_placement(&[block]).unwrap().world
}

/// The plate's top and the turned block (picked whole, seen from above) become their XY planes;
/// the next press turns the block level (Parallel), the next sets its XY on the plate's (coincident,
/// still facing up), and clears the picks.
#[test]
fn faces_and_parts_become_planes_then_parallel_then_coincident() {
    let (mut h, block) = plate_and_turned_block();
    look_down(&mut h.app);
    let top = face(&h.app, Vec3::new(5.0, 5.0, 10.0));
    h.app.session.selection = vec![top, Sel::Occurrence { path: vec![block] }];
    crate::keymap::press(&mut h.app, "Shift+P", false);
    assert_eq!(h.app.session.selection, vec![part_plane(&h, "Plate:1", "XY"), part_plane(&h, "Block:1", "XY")]);

    h.app.start("ui.plane_mate");
    let j = h.app.session.doc.assembly.joints.first().expect("a constraint");
    assert_eq!(j.constraint, Some(Constraint::Parallel));
    let w = block_world(&h, block);
    assert!(w[2][2] > 1.0 - 1e-6, "the block is level: {:?}", w[2]);
    assert_eq!(h.app.session.selection.len(), 2, "kept for the next press");

    h.app.start("ui.plane_mate");
    let j = h.app.session.doc.assembly.joints.first().expect("a constraint");
    assert_eq!(h.app.session.doc.assembly.joints.len(), 1, "the parallel became it");
    assert!(matches!(j.constraint, Some(Constraint::Flush | Constraint::Mate)), "{:?}", j.constraint);
    let w = block_world(&h, block);
    assert!(w[2][2] > 1.0 - 1e-6 && w[3][2].abs() < 1e-6, "its XY on the plate's: {:?}", w);
    assert!(h.app.session.selection.is_empty());
}

/// Nothing picked: the design's plane facing the screen. One plane of a part: the design's plane
/// most parallel to it joins it. Two planes of one part: refused, nothing made.
#[test]
fn nothing_picked_one_plane_and_one_part() {
    let (mut h, _) = plate_and_turned_block();
    look_down(&mut h.app);
    h.app.session.selection.clear();
    h.app.start("ui.plane_mate");
    assert_eq!(h.app.session.selection, vec![plane("XY")]);

    h.app.session.selection = vec![plane("Plate:1/YZ")];
    h.app.start("ui.plane_mate");
    assert_eq!(h.app.session.selection, vec![plane("Plate:1/YZ"), plane("YZ")]);

    h.app.session.selection = vec![plane("Plate:1/YZ"), plane("Plate:1/XZ")];
    h.app.start("ui.plane_mate");
    assert!(h.app.session.doc.assembly.joints.is_empty());
}

/// As a person does it: the pointer over the view, the faces picked, Shift+P pressed as a key
/// with the window drawing between presses.
#[test]
fn shift_p_with_frames_between_presses_mates_the_planes() {
    let (mut h, block) = plate_and_turned_block();
    look_down(&mut h.app);
    let c = h.app.viewport.rect.expect("a view").center();
    h.step(&json!({"move": [c.x, c.y]})).unwrap();
    let top = face(&h.app, Vec3::new(5.0, 5.0, 10.0));
    h.app.session.selection = vec![top, Sel::Occurrence { path: vec![block] }];
    h.frames(2);
    let press = |h: &mut Harness| {
        h.step(&json!({"key": "P", "shift": true})).unwrap();
        h.frames(3);
    };
    press(&mut h);
    assert_eq!(h.app.session.selection, vec![part_plane(&h, "Plate:1", "XY"), part_plane(&h, "Block:1", "XY")], "status: {:?}", h.app.status);
    press(&mut h);
    assert_eq!(h.app.session.doc.assembly.joints.len(), 1, "parallel made; status: {:?}", h.app.status);
    press(&mut h);
    assert!(matches!(h.app.session.doc.assembly.joints[0].constraint, Some(Constraint::Flush | Constraint::Mate)));
}

/// Two copies of one part (1:1 and 1:2), the second turned 10° and free: Plane Mate's Parallel
/// levels the copy, and it stays free to slide (not locked to the first).
#[test]
fn two_copies_of_one_part_go_parallel_and_stay_free() {
    let mut s = Session::default();
    let mut run = |id: &str, p: Value| s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"));
    run("component.create", json!({"name": "1"}));
    run("solid.box", json!({"length": 20, "width": 20, "height": 20}));
    run("component.activate", json!({"component": "root"}));
    run("occurrence.copy", json!({"component": "1", "translate": [60, 0, 0]}));
    drop(run);
    let occs: Vec<u64> = s.doc.occurrences.iter().map(|o| o.id).collect();
    assert_eq!(occs.len(), 2, "two copies");
    let (a, b) = (occs[0], occs[1]);
    s.execute("occurrence.ground", &json!({"occurrence": a, "grounded": true})).unwrap();
    s.execute("occurrence.drag", &json!({"occurrence": b, "rotate": {"origin": [70, 10, 10], "axis": [1, 0, 0], "angle": 10f64.to_radians()}})).unwrap();
    let mut h = Harness::new();
    *h.app = SolveApp::new(s, Services::default());
    h.app.home.open = false;
    h.frames(3);
    look_down(&mut h.app);
    let c = h.app.viewport.rect.expect("a view").center();
    h.step(&json!({"move": [c.x, c.y]})).unwrap();
    h.app.session.selection = vec![face(&h.app, Vec3::new(10.0, 10.0, 20.0)), Sel::Occurrence { path: vec![b] }];
    h.frames(2);
    let press = |h: &mut Harness| {
        h.step(&json!({"key": "P", "shift": true})).unwrap();
        h.frames(3);
    };
    press(&mut h);
    assert_eq!(h.app.session.selection.len(), 2, "two planes: {:?} / {:?}", h.app.session.selection, h.app.status);
    press(&mut h);
    assert_eq!(h.app.session.doc.assembly.joints.len(), 1, "status: {:?}", h.app.status);
    let w = h.app.session.occurrence_placement(&[b]).unwrap().world;
    assert!(w[2][2] > 1.0 - 1e-6, "the copy is level: {:?}", w[2]);
    // Still free to slide (held to the grounded copy, it moves as far as its joint allows).
    let before = w[3];
    let c = solidcraft_engine::doc::apply_point(&w, Vec3::new(10.0, 10.0, 10.0));
    h.app.session.execute("joint.reach", &json!({"occurrence": b, "at": [c.x, c.y, c.z], "point": [c.x + 10.0, c.y, c.z]})).unwrap();
    let after = h.app.session.occurrence_placement(&[b]).unwrap().world[3];
    assert!((after[0] - before[0]).abs() > 5.0, "slides: {before:?} -> {after:?}");
}

/// The Browser key of origin plane `item` of the top-level part named `occurrence`.
fn part_plane(h: &Harness, occurrence: &str, item: &str) -> Sel {
    let id = h.app.session.doc.occurrences.iter().find(|o| o.name == occurrence).expect("the part").id;
    Sel::Plane { name: solidcraft_engine::origins::key(&[id], item) }
}

/// The planes the first press picks are drawn while selected, though a part's planes are hidden.
#[test]
fn the_planes_picked_are_drawn_while_selected() {
    let (mut h, block) = plate_and_turned_block();
    look_down(&mut h.app);
    let top = face(&h.app, Vec3::new(5.0, 5.0, 10.0));
    h.app.session.selection = vec![top, Sel::Occurrence { path: vec![block] }];
    crate::keymap::press(&mut h.app, "Shift+P", false);
    let shown = crate::viewport::selected_hidden_origins(&h.app);
    assert_eq!(shown.len(), 2, "both planes drawn: {shown:?}");
}
