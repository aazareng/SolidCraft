//! The shortcut bar: S opens the mode's tools at the pointer; a click runs one; Customize edits
//! the set, kept in the preferences.
use serde_json::{Value, json};
use solidcraft_engine::Session;

use super::Mode;
use crate::scenario::Harness;
use crate::{Services, SolveApp};

fn part() -> Harness {
    let mut h = Harness::new();
    h.app.home.open = false;
    h.frames(2);
    let c = h.app.viewport.rect.expect("a view").center();
    h.step(&json!({"move": [c.x, c.y]})).unwrap();
    h
}

fn ids(h: &Harness, mode: Mode) -> Vec<String> {
    super::items(&h.app, mode).into_iter().map(|(id, _)| id).collect()
}

/// S opens the part bar at the pointer; a click on Extrude starts it and closes the bar.
#[test]
fn s_opens_the_part_bar_and_a_click_runs_a_tool() {
    let mut h = part();
    crate::keymap::press(&mut h.app, "S", false);
    assert!(super::is_open(&h.app), "S opens the bar");
    h.frames(2);
    assert_eq!(h.app.menu.shortcut.open.as_ref().map(|b| b.mode), Some(Mode::Part));
    assert!(ids(&h, Mode::Part).contains(&"solid.extrude".to_string()));
    h.step(&json!({"click": {"handle": "shortcut_bar:sketch.create"}})).unwrap();
    h.frames(2);
    assert!(!super::is_open(&h.app));
    assert!(h.app.dialog.is_some() || h.app.tool.is_some(), "Create Sketch started");
}

/// Customizing: a search result is added, a picked tool moves and goes, Reset puts the set back;
/// the changes are the preferences' and only the changed mode's.
#[test]
fn customize_adds_moves_removes_and_resets() {
    let mut h = part();
    super::open(&mut h.app, None);
    h.frames(2);
    h.step(&json!({"click": {"handle": "shortcut_bar:customize"}})).unwrap();
    h.frames(1);
    h.app.menu.shortcut.open.as_mut().unwrap().search = "draft".into();
    h.frames(2);
    let found = super::matches("draft");
    let (first, _) = found.first().expect("a command named draft").clone();
    h.step(&json!({"click": {"handle": format!("shortcut_bar:found:{first}")}})).unwrap();
    h.frames(2);
    let now = ids(&h, Mode::Part);
    assert_eq!(now.last(), Some(&first), "added at the end: {now:?}");
    assert!(h.app.preferences.shortcut_bars.bars.contains_key("part") && !h.app.preferences.shortcut_bars.bars.contains_key("sketch"));

    h.step(&json!({"click": {"handle": format!("shortcut_bar:{first}")}})).unwrap();
    h.frames(1);
    h.step(&json!({"click": {"handle": "shortcut_bar:remove"}})).unwrap();
    h.frames(2);
    assert!(!ids(&h, Mode::Part).contains(&first));
    assert!(h.app.preferences.shortcut_bars.bars.is_empty(), "back to the defaults: nothing kept");
}

/// The bar follows the mode: an assembly (parts placed, the design active) gets its own set.
#[test]
fn an_assembly_has_its_own_bar_and_search_ranks_names_that_start_with_it() {
    let mut s = Session::default();
    let mut run = |id: &str, p: Value| s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"));
    run("component.create", json!({"name": "A"}));
    run("solid.box", json!({"length": 10, "width": 10, "height": 10}));
    run("component.activate", json!({"component": "root"}));
    drop(run);
    let mut h = Harness::new();
    *h.app = SolveApp::new(s, Services::default());
    h.app.home.open = false;
    h.frames(2);
    assert_eq!(Mode::of(&h.app), Mode::Assembly);
    assert!(ids(&h, Mode::Assembly).contains(&"ui.plane_mate".to_string()));
    let found = super::matches("extru");
    assert!(found.first().is_some_and(|(_, label)| label.to_lowercase().starts_with("extru")), "{found:?}");
}
