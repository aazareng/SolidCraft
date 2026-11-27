//! Two picks on two parts open the quick constrain bar; a rule on it constrains them.
use serde_json::{Value, json};
use solidcore_record::Vec3;
use solidcraft_engine::doc::joints::Constraint;
use solidcraft_engine::{Sel, Session};

use crate::scenario::Harness;
use crate::{Services, SolveApp};

/// A grounded 100 mm plate (top at z = 10) and a free 20 mm block above it (bottom at z = 50).
fn plate_and_block() -> Harness {
    let mut s = Session::default();
    let mut run = |id: &str, p: Value| s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"));
    for (name, size, corner) in [("Plate", [100, 100, 10], [0, 0, 0]), ("Block", [20, 20, 20], [30, 40, 50])] {
        run("component.activate", json!({"component": "root"}));
        run("component.create", json!({ "name": name }));
        run("solid.box", json!({"length": size[0], "width": size[1], "height": size[2], "corner": corner}));
    }
    run("component.activate", json!({"component": "root"}));
    run("occurrence.ground", json!({"occurrence": "Plate:1", "grounded": true}));
    let mut h = Harness::new();
    *h.app = SolveApp::new(s, Services::default());
    h.app.home.open = false;
    h.frames(3);
    // The pointer over the view, where the person is picking.
    let c = h.app.viewport.rect.expect("a view").center();
    h.step(&json!({"move": [c.x, c.y]})).unwrap();
    h
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

fn select(h: &mut Harness, points: &[Vec3]) {
    let picks: Vec<Sel> = points.iter().map(|p| face(&h.app, *p)).collect();
    h.app.session.selection = picks;
    h.frames(2);
}

fn rules(h: &Harness) -> Vec<Constraint> {
    let rules = h.app.menu.constrain.open.as_ref().map(|b| b.rules.clone()).unwrap_or_default();
    rules.into_iter().filter_map(|r| if let super::Rule::Constraint(c) = r { Some(c) } else { None }).collect()
}

fn the_rule(h: &Harness) -> Option<super::Rule> {
    h.app.menu.constrain.open.as_ref().and_then(|b| b.rules.first().copied())
}

/// Where the placement of `name` puts the point `local` of its part.
fn placed(h: &Harness, name: &str, local: Vec3) -> Vec3 {
    let o = h.app.session.doc.occurrences.iter().find(|o| o.name == name).unwrap().id;
    solidcraft_engine::doc::apply_point(&h.app.session.occurrence_placement(&[o]).unwrap().world, local)
}

/// Four faces, two on each part: Width centres the block between the plate's sides in one undo
/// step; three with a plane of the plate: Symmetric mirrors a plate face onto the block.
#[test]
fn four_faces_make_width_and_three_make_symmetric() {
    let mut h = plate_and_block();
    let undo = h.app.session.undo.len();
    // The plate's sides x = 0 and x = 100, the block's x = 30 and x = 50.
    select(&mut h, &[Vec3::new(0.0, 50.0, 5.0), Vec3::new(100.0, 50.0, 5.0), Vec3::new(30.0, 50.0, 60.0), Vec3::new(50.0, 50.0, 60.0)]);
    assert_eq!(the_rule(&h), Some(super::Rule::Width));
    h.step(&json!({"click": {"handle": "dialog:quick_constrain:Width"}})).unwrap();
    h.frames(3);
    assert!(h.app.menu.constrain.open.as_ref().is_none_or(|b| b.error.is_none()), "{:?}", h.app.menu.constrain.open.as_ref().and_then(|b| b.error.clone()));
    // The block's middle (x = 40) is now at the plate's (x = 50), and it is not turned over.
    let mid = placed(&h, "Block:1", Vec3::new(40.0, 50.0, 60.0));
    assert!((mid.x - 50.0).abs() < 1e-6, "{mid:?}");
    let top = placed(&h, "Block:1", Vec3::new(40.0, 50.0, 70.0));
    assert!(top.z > mid.z, "the block keeps its top up: {mid:?} {top:?}");
    assert_eq!(h.app.session.undo.len(), undo + 1, "one undo step");
    assert_eq!(h.app.session.undo.last().map(|s| s.label.as_str()), Some("Width"));
    h.app.run("edit.undo", json!({})).unwrap();
    assert!(h.app.session.doc.assembly.joints.is_empty(), "undo takes all of it back");
    h.frames(2);

    // Symmetric about the plate's own midplane (x = 50) between the plate's side x = 100 and the
    // block's side x = 30: that side goes to x = 0, the mirror of x = 100.
    h.app.run("component.activate", json!({"component": "Plate", "occurrence": "Plate:1"})).unwrap();
    h.app.run("construct.plane.midplane", json!({"a": {"face": [0, 50, 5]}, "b": {"face": [100, 50, 5]}, "name": "Middle"})).unwrap();
    h.app.run("component.activate", json!({"component": "root"})).unwrap();
    h.frames(2);
    let mut picks: Vec<Sel> = [Vec3::new(100.0, 50.0, 5.0), Vec3::new(30.0, 50.0, 60.0)].iter().map(|p| face(&h.app, *p)).collect();
    picks.push(Sel::Plane { name: "Plate:1/Middle".into() });
    // The person picks in the view again (the bar that made Width stays under the pointer).
    let c = h.app.viewport.rect.expect("a view").center();
    h.step(&json!({"move": [c.x - 200.0, c.y - 150.0]})).unwrap();
    h.app.session.selection = picks;
    h.frames(2);
    assert_eq!(the_rule(&h), Some(super::Rule::Symmetric));
    h.step(&json!({"click": {"handle": "dialog:quick_constrain:Symmetric"}})).unwrap();
    h.frames(3);
    let side = placed(&h, "Block:1", Vec3::new(30.0, 50.0, 60.0));
    assert!(side.x.abs() < 1e-6, "{side:?}");
}

/// The plate's top and the block's bottom face each other: the bar opens with Mate first, and a
/// click on it seats the block on the plate, clears the picks and closes the bar.
#[test]
fn two_facing_faces_offer_mate_first_and_it_seats_the_part() {
    let mut h = plate_and_block();
    select(&mut h, &[Vec3::new(5.0, 5.0, 10.0), Vec3::new(31.0, 41.0, 50.0)]);
    assert!(crate::quick_constrain::is_open(&h.app), "the bar opens by itself");
    let r = rules(&h);
    assert_eq!(r.first(), Some(&Constraint::Mate), "{r:?}");
    assert!(r.contains(&Constraint::Distance) && r.contains(&Constraint::Parallel), "{r:?}");
    let label = crate::dialogs_constrain::label(Constraint::Mate);
    h.step(&json!({"click": {"handle": format!("dialog:quick_constrain:{label}")}})).unwrap();
    h.frames(3);
    assert_eq!(h.app.session.doc.assembly.joints.first().and_then(|j| j.constraint), Some(Constraint::Mate));
    assert!(h.app.session.selection.is_empty(), "the picks are used up");
    assert!(h.app.menu.constrain.open.as_ref().is_some_and(|b| b.made.is_some()), "the bar stays to flip what it made");
    // The block's bottom now lies on the plate's top.
    let block = h.app.session.doc.occurrences.iter().find(|o| o.name == "Block:1").unwrap().id;
    let w = h.app.session.occurrence_placement(&[block]).unwrap().world;
    assert!((solidcraft_engine::doc::apply_point(&w, Vec3::new(30.0, 40.0, 50.0)).z - 10.0).abs() < 1e-6);
    // Flip turns the block over on the same faces, without the dialog; Delete takes it away.
    let flip = h.app.session.doc.assembly.joints[0].flip;
    h.step(&json!({"click": {"handle": "quick_constrain:made:flip"}})).unwrap();
    h.frames(3);
    assert_ne!(h.app.session.doc.assembly.joints[0].flip, flip, "flipped");
    let w = h.app.session.occurrence_placement(&[block]).unwrap().world;
    assert!(w[2][2] < -0.5, "turned over: {:?}", w[2]);
    h.step(&json!({"click": {"handle": "quick_constrain:made:delete"}})).unwrap();
    h.frames(2);
    assert!(h.app.session.doc.assembly.joints.is_empty() && !crate::quick_constrain::is_open(&h.app));
}

/// Two faces looking the same way (both tops) are flush first; two faces of one part open nothing.
#[test]
fn same_way_faces_are_flush_first_and_one_part_opens_nothing() {
    let mut h = plate_and_block();
    select(&mut h, &[Vec3::new(5.0, 5.0, 10.0), Vec3::new(31.0, 41.0, 70.0)]);
    assert_eq!(rules(&h).first(), Some(&Constraint::Flush), "{:?}", rules(&h));
    select(&mut h, &[Vec3::new(5.0, 5.0, 10.0), Vec3::new(5.0, 5.0, 0.0)]);
    assert!(!crate::quick_constrain::is_open(&h.app), "both on the plate");
    select(&mut h, &[Vec3::new(5.0, 5.0, 10.0)]);
    assert!(!crate::quick_constrain::is_open(&h.app), "one pick");
}

/// Distance shows the gap the two have now (40 mm) to change; Apply adds it with the typed value.
#[test]
fn distance_takes_its_value_from_the_bar() {
    let mut h = plate_and_block();
    select(&mut h, &[Vec3::new(5.0, 5.0, 10.0), Vec3::new(31.0, 41.0, 50.0)]);
    let label = crate::dialogs_constrain::label(Constraint::Distance);
    h.step(&json!({"click": {"handle": format!("dialog:quick_constrain:{label}")}})).unwrap();
    h.frames(2);
    let bar = h.app.menu.constrain.open.as_mut().expect("still open for the value");
    assert_eq!(bar.offset, "40 mm");
    bar.offset = "25 mm".into();
    h.step(&json!({"click": {"handle": "quick_constrain:apply"}})).unwrap();
    h.frames(3);
    let j = h.app.session.doc.assembly.joints.first().expect("a constraint");
    assert_eq!(j.constraint, Some(Constraint::Distance));
    assert!((j.offset.abs() - 25.0).abs() < 1e-6, "{}", j.offset);
    // Reverse puts the gap on the other side, in place.
    let before = j.offset;
    h.step(&json!({"click": {"handle": "quick_constrain:made:reverse"}})).unwrap();
    h.frames(3);
    let j = h.app.session.doc.assembly.joints.first().expect("still there");
    assert!((j.offset + before).abs() < 1e-6, "{} after {before}", j.offset);
    assert_eq!(h.app.menu.constrain.open.as_ref().and_then(|b| b.made.as_ref()).map(|m| m.value.as_str()), Some("-25 mm"));
}
