//! The pick card: one pick on a placed part shows its trail, what made it and its joints.
use serde_json::{Value, json};
use solidcore_record::Vec3;
use solidcraft_engine::{Sel, Session};

use super::RowKind;
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

/// A grounded plate and a block mated onto it, built by `more` after.
fn assembly(more: impl FnOnce(&mut Session)) -> Harness {
    let mut s = Session::default();
    let mut run = |id: &str, p: Value| s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"));
    for (name, size, corner) in [("Plate", [100, 100, 10], [0, 0, 0]), ("Block", [20, 20, 20], [30, 40, 50])] {
        run("component.activate", json!({"component": "root"}));
        run("component.create", json!({ "name": name }));
        run("solid.box", json!({"length": size[0], "width": size[1], "height": size[2], "corner": corner}));
    }
    run("component.activate", json!({"component": "root"}));
    run("occurrence.ground", json!({"occurrence": "Plate:1", "grounded": true}));
    run("assembly.constraint.create", json!({"type": "mate", "a": {"occurrence": "Plate:1", "face": [5, 5, 10]}, "b": {"occurrence": "Block:1", "face": [31, 41, 50]}}));
    drop(run);
    more(&mut s);
    let mut h = Harness::new();
    *h.app = SolveApp::new(s, Services::default());
    h.app.home.open = false;
    h.frames(3);
    let c = h.app.viewport.rect.expect("a view").center();
    h.step(&json!({"move": [c.x, c.y]})).unwrap();
    h
}

fn pick(h: &mut Harness, p: Vec3) {
    let f = face(&h.app, p);
    h.app.session.selection = vec![f];
    h.frames(2);
}

fn joint_rows(h: &Harness) -> Vec<super::Row> {
    h.app.menu.card.open.as_ref().map(|c| c.joints.clone()).unwrap_or_default()
}

/// The block's top picked: the card lists its mate; Suppress strikes it through and keeps the card
/// open; Delete takes it away.
#[test]
fn a_part_shows_its_joints_to_suppress_and_delete() {
    let mut h = assembly(|_| {});
    pick(&mut h, Vec3::new(40.0, 50.0, 30.0));
    assert!(super::is_open(&h.app), "the card opens by itself");
    let card = h.app.menu.card.open.as_ref().unwrap();
    let trail: Vec<&str> = card.trail.iter().map(|c| c.label.as_str()).collect();
    assert_eq!((trail.first(), trail.last()), (Some(&"Face"), Some(&"Block:1")), "the face, its feature, its part: {trail:?}");
    assert!(trail.len() == 3, "the box feature that made it between: {trail:?}");
    let rows = joint_rows(&h);
    assert_eq!(rows.len(), 1, "{rows:?}");
    let id = rows[0].id;
    // The icon opens the cross: Suppress and Delete are under it.
    h.step(&json!({"click": {"handle": format!("pick_card:joint:{id}:rule")}})).unwrap();
    h.frames(1);
    h.step(&json!({"click": {"handle": format!("pick_card:joint:{id}:suppress")}})).unwrap();
    h.frames(2);
    assert!(h.app.session.doc.assembly.joints.iter().any(|j| j.id == id && j.suppressed));
    assert!(joint_rows(&h).first().is_some_and(|r| r.suppressed), "still open, now suppressed");
    h.step(&json!({"click": {"handle": format!("pick_card:joint:{id}:rule")}})).unwrap();
    h.frames(1);
    h.step(&json!({"click": {"handle": format!("pick_card:joint:{id}:delete")}})).unwrap();
    h.frames(2);
    assert!(h.app.session.doc.assembly.joints.is_empty());
}

/// Flip sits over Edit in a mate's cross: a click turns the block over on the plate in one step and
/// keeps the card open.
#[test]
fn a_mates_cross_flips_it() {
    let mut h = assembly(|_| {});
    pick(&mut h, Vec3::new(40.0, 50.0, 30.0));
    let id = joint_rows(&h).first().expect("the mate's row").id;
    let state = |h: &Harness| h.app.session.doc.assembly.joints.iter().find(|j| j.id == id).map(|j| (j.flip, j.constraint));
    let (before, undo) = (state(&h), h.app.session.undo.len());
    h.step(&json!({"click": {"handle": format!("pick_card:joint:{id}:rule")}})).unwrap();
    h.frames(1);
    let flip = crate::scenario::handle_at(&format!("pick_card:joint:{id}:flip")).expect("Flip in the cross");
    let edit = crate::scenario::handle_at(&format!("pick_card:joint:{id}:edit")).expect("Edit in the cross");
    assert!(flip.y < edit.y && (flip.x - edit.x).abs() < 1.0, "Flip over Edit: {flip:?} / {edit:?}");
    h.step(&json!({"click": {"handle": format!("pick_card:joint:{id}:flip")}})).unwrap();
    h.frames(2);
    assert_ne!(state(&h), before, "it flipped");
    assert_eq!(h.app.session.undo.len(), undo + 1, "one undo step");
    assert!(super::is_open(&h.app), "the card stays");
}

/// A copy a component pattern placed: the card shows the pattern under Made by; a click on its
/// name selects it.
#[test]
fn a_patterned_copy_shows_its_pattern() {
    let mut h = assembly(|s| {
        s.execute("solid.pattern.rectangular", &json!({"components": ["Block"], "dir1": [1, 0, 0], "count1": 2, "spacing1": 40})).unwrap();
    });
    // The copy sits 40 mm along x from the block (whose top is at z = 30 after the mate).
    let copy_top = h.app.session.doc.occurrences.iter().filter(|o| o.name.starts_with("Block")).count();
    assert_eq!(copy_top, 2, "a copy was placed");
    let tops: Vec<Vec3> = [40.0, 80.0].iter().map(|x| Vec3::new(*x, 50.0, 30.0)).collect();
    let mut seen = false;
    for p in tops {
        pick(&mut h, p);
        let made: Vec<_> = h.app.menu.card.open.as_ref().map(|c| c.made_by.clone()).unwrap_or_default();
        if let Some(row) = made.first() {
            assert_eq!(row.kind, RowKind::Feature);
            h.step(&json!({"click": {"handle": format!("pick_card:feature:{}", row.id)}})).unwrap();
            h.frames(1);
            assert_eq!(h.app.session.selection, vec![Sel::Feature { id: row.id }]);
            seen = true;
            break;
        }
    }
    assert!(seen, "a block shows the pattern");
}

/// In a part (nothing placed), a face picked shows breadcrumbs along the top of the view: the
/// extrude's sketch, the extrude, the face. A click on the sketch's crumb selects it.
#[test]
fn breadcrumbs_show_a_parts_face_feature_and_sketch() {
    let mut h = Harness::new();
    h.app.home.open = false;
    for (id, p) in [
        ("sketch.create", json!({"plane": "XY", "name": "Base"})),
        ("sketch.rectangle.two_point", json!({"p0": [0, 0], "p1": [60, 20]})),
        ("sketch.finish", json!({})),
        ("solid.extrude", json!({"distance": 10, "name": "Slab"})),
    ] {
        h.app.run(id, p).unwrap_or_else(|e| panic!("{id}: {e}"));
    }
    h.frames(2);
    pick(&mut h, Vec3::new(30.0, 10.0, 10.0));
    let cols = h.app.menu.card.crumbs.as_ref().map(|(_, c)| c.clone()).unwrap_or_default();
    let main: Vec<&str> = cols.iter().map(|c| c.main.label.as_str()).collect();
    assert_eq!((main.first(), main.get(1)), (Some(&"Face"), Some(&"Slab")), "the pick, then its feature: {main:?}");
    assert_eq!(cols[1].below.iter().map(|c| c.label.as_str()).collect::<Vec<_>>(), vec!["Base"], "the driving sketch under the feature");
    assert_eq!(cols.last().map(|c| c.below.len()), Some(4), "the design's three planes and origin under it: {cols:?}");
    assert!(!super::is_open(&h.app), "no card in a part: nothing placed");
    // Below the ribbon and right of the Browser, in the view.
    let grip = crate::scenario::handle_at("breadcrumb:grip").expect("the trail is drawn");
    assert!(h.app.ui.ribbon_bottom > 0.0 && grip.y > h.app.ui.ribbon_bottom, "below the ribbon: {grip:?} / {}", h.app.ui.ribbon_bottom);
    let middle = h.app.viewport.layout.as_ref().map(|l| l.middle).expect("a layout");
    assert!(grip.x > middle.left(), "right of the Browser: {grip:?} / {middle:?}");
    // The sketch under the feature (column 1, first item below it) selects the sketch.
    h.step(&json!({"click": {"handle": "breadcrumb:1:0"}})).unwrap();
    h.frames(1);
    let base = h.app.session.doc.features.iter().find(|f| f.name == "Base").unwrap().id;
    assert_eq!(h.app.session.selection, vec![Sel::Feature { id: base }]);
}

/// What the breadcrumbs say the selection measures, once it is in.
fn measured(h: &mut Harness) -> Option<String> {
    for _ in 0..300 {
        crate::quick_measure::poll(&mut h.app, &h.ctx);
        if let Some(s) = h.app.quick_measure.summary(&h.app) {
            h.frames(1);
            return Some(s);
        }
        h.frames(1);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    None
}

/// The breadcrumbs run from the design on the left to the pick on the right, and the pick shows
/// what it measures (a face its area) with Quick measurement off. Two parallel faces show their
/// distance, and no angle.
#[test]
fn breadcrumbs_put_the_pick_on_the_right_with_what_it_measures() {
    let mut h = Harness::new();
    h.app.home.open = false;
    assert!(!h.app.preferences.quick_measure);
    for (id, p) in [
        ("sketch.create", json!({"plane": "XY", "name": "Base"})),
        ("sketch.rectangle.two_point", json!({"p0": [0, 0], "p1": [60, 20]})),
        ("sketch.finish", json!({})),
        ("solid.extrude", json!({"distance": 10, "name": "Slab"})),
    ] {
        h.app.run(id, p).unwrap_or_else(|e| panic!("{id}: {e}"));
    }
    h.frames(2);
    pick(&mut h, Vec3::new(30.0, 10.0, 10.0));
    let area = measured(&mut h).expect("the face is measured");
    assert!(area.contains("1200") || area.contains("1,200"), "60 × 20: {area}");
    let n = h.app.menu.card.crumbs.as_ref().map_or(0, |(_, c)| c.len());
    assert!(n >= 3, "face, feature, design");
    let pick_at = crate::scenario::handle_at("breadcrumb:0").expect("the pick is drawn");
    let design_at = crate::scenario::handle_at(&format!("breadcrumb:{}", n - 1)).expect("the design is drawn");
    assert!(pick_at.x > design_at.x, "the pick on the right: {pick_at:?} / {design_at:?}");
    assert!(crate::scenario::rect_of("measure:quick").is_none(), "no corner readout with its preference off");
    // The top and the bottom: 10 mm apart, parallel.
    h.app.session.selection = vec![face(&h.app, Vec3::new(30.0, 10.0, 10.0)), face(&h.app, Vec3::new(30.0, 10.0, 0.0))];
    let gap = measured(&mut h).expect("the pair is measured");
    assert!(gap.contains("10") && !gap.contains('°'), "{gap}");
    let cols = h.app.menu.card.crumbs.as_ref().map(|(_, c)| c.clone()).unwrap_or_default();
    assert_eq!(cols.iter().map(|c| c.main.label.as_str()).collect::<Vec<_>>(), vec!["Face + Face"]);
    assert!(crate::scenario::handle_at("breadcrumb:0").is_some(), "drawn once measured");
}

/// In an assembly the part's column has its mate over it and its planes and origin under it; a
/// click on a plane selects that plane of the part.
#[test]
fn breadcrumbs_show_a_parts_mates_planes_and_origin() {
    let mut h = assembly(|_| {});
    pick(&mut h, Vec3::new(40.0, 50.0, 30.0));
    let cols = h.app.menu.card.crumbs.as_ref().map(|(_, c)| c.clone()).unwrap_or_default();
    let (i, part) = cols.iter().enumerate().find(|(_, c)| c.main.label == "Block:1").expect("the part's column");
    assert_eq!(part.above.len(), 1, "its mate over it");
    let below: Vec<&str> = part.below.iter().map(|c| c.label.as_str()).collect();
    assert_eq!(below.len(), 4, "three planes and the origin: {below:?}");
    h.step(&json!({"click": {"handle": format!("breadcrumb:{i}:0")}})).unwrap();
    h.frames(1);
    assert!(matches!(h.app.session.selection.as_slice(), [Sel::Plane { .. }]), "{:?}", h.app.session.selection);
    // A part's planes are off by default: the selected one is drawn while it stays selected.
    let shown = crate::viewport::selected_hidden_origins(&h.app);
    assert!(matches!(shown.as_slice(), [(_, crate::viewport::HoveredOrigin::Plane(_))]), "{shown:?}");
    h.app.session.selection.clear();
    assert!(crate::viewport::selected_hidden_origins(&h.app).is_empty());
}

/// The rules a constraint's icon offers: a mate between two planes can loosen to flush (as
/// strict), distance, then parallel, square or an angle, and tighten to nothing; parallel tightens
/// to distance and then mate or flush.
#[test]
fn a_mates_icon_offers_looser_and_stricter_rules() {
    use solidcraft_engine::doc::joints::{Constraint, RefShape};
    let (looser, stricter) = super::variants(Constraint::Mate, RefShape::Plane, RefShape::Plane);
    assert!(stricter.is_empty(), "{stricter:?}");
    assert_eq!(looser.first(), Some(&Constraint::Flush), "{looser:?}");
    assert!(looser.contains(&Constraint::Distance) && looser.contains(&Constraint::Parallel) && looser.contains(&Constraint::Angle), "{looser:?}");
    let (_, stricter) = super::variants(Constraint::Parallel, RefShape::Plane, RefShape::Plane);
    assert_eq!(stricter.first(), Some(&Constraint::Distance), "{stricter:?}");
    assert!(stricter.contains(&Constraint::Mate) && stricter.contains(&Constraint::Flush), "{stricter:?}");
}

/// On the card, a click on the mate's icon shows its other rules; a click on Parallel makes it
/// parallel, in place, and the card stays to switch it back.
#[test]
fn a_mate_downgrades_to_parallel_and_back_in_two_clicks() {
    use solidcraft_engine::doc::joints::Constraint;
    let mut h = assembly(|_| {});
    pick(&mut h, Vec3::new(40.0, 50.0, 30.0));
    let id = joint_rows(&h)[0].id;
    let rule = |h: &Harness| h.app.session.doc.assembly.joints.iter().find(|j| j.id == id).and_then(|j| j.constraint);
    assert_eq!(rule(&h), Some(Constraint::Mate));
    let open_cross = |h: &mut Harness| {
        h.step(&json!({"click": {"handle": format!("pick_card:joint:{id}:rule")}})).unwrap();
        h.frames(1);
    };
    open_cross(&mut h);
    assert_eq!(h.app.menu.card.open.as_ref().and_then(|c| c.variants), Some((RowKind::Joint, id)), "the cross shows");
    h.step(&json!({"click": {"handle": "pick_card:variant:parallel"}})).unwrap();
    h.frames(2);
    assert_eq!(rule(&h), Some(Constraint::Parallel), "status: {:?}", h.app.status);
    // As a new mate is: the quick bar's Flip, value and Delete by the pointer.
    let made = crate::quick_constrain::made_name(&h.app);
    assert!(made.is_some(), "Flip is offered for the switched mate");
    let flip = h.app.session.doc.assembly.joints.iter().find(|j| j.id == id).map(|j| j.flip);
    h.step(&json!({"click": {"handle": "quick_constrain:made:flip"}})).unwrap();
    h.frames(2);
    assert_ne!(h.app.session.doc.assembly.joints.iter().find(|j| j.id == id).map(|j| j.flip), flip, "flipped");
    // The part picked again: its card, and the mate made a mate again from the cross.
    h.app.session.selection.clear();
    h.frames(2);
    let c = h.app.viewport.rect.expect("a view").center();
    h.step(&json!({"move": [c.x, c.y]})).unwrap();
    pick(&mut h, Vec3::new(40.0, 50.0, 30.0));
    if !super::is_open(&h.app) {
        // The flip turned the block: pick its top where it is now.
        let w = h.app.session.occurrence_placement(&[h.app.session.doc.occurrences.iter().find(|o| o.name == "Block:1").unwrap().id]).unwrap().world;
        let top = solidcraft_engine::doc::apply_point(&w, Vec3::new(40.0, 50.0, 70.0));
        pick(&mut h, top);
    }
    open_cross(&mut h);
    h.step(&json!({"click": {"handle": "pick_card:variant:mate"}})).unwrap();
    h.frames(2);
    assert_eq!(rule(&h), Some(Constraint::Mate), "back to a mate");
}
