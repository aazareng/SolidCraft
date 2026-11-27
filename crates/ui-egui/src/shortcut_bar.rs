//! The shortcut bar: a key (S) opens a bar of tool icons at the pointer, a set for each mode the
//! person can be in (part, assembly, sketch, drawing, and the Surface, Sheet Metal, Plastic, Mesh
//! and Render workspaces), with a search box to find any other command.
//!
//! - Each mode starts with the tools the mouse-gesture ring has there (the specials such as
//!   Repeat and Undo included), and the person's own changes are kept in the preferences.
//! - Customize (the gear at the end) turns the bar into its editor: a click on a tool picks it,
//!   the arrows move it and × removes it; a search result is added rather than run; Reset puts
//!   the mode's set back.
//! - A click on a tool runs it as a menu pick would (`context_menu::run_item`) and closes the
//!   bar. Esc, a click elsewhere, or the pointer moving well away from it also close it.

use std::collections::BTreeMap;

use egui::{Pos2, Rect, Sense, vec2};
use serde::{Deserialize, Serialize};

use crate::SolveApp;
use crate::mouse_gesture::Studio;
use crate::ui_kit::{Container, Item, Surface};

/// The bar's default key.
pub const KEY: &str = "S";
/// One tool's square, in points.
const CELL: f32 = 30.0;
/// Tools on one row before the next begins.
const PER_ROW: usize = 10;
/// The most tools one bar keeps.
const MAX_TOOLS: usize = 40;
/// Search results shown at most.
const MATCHES: usize = 8;
/// How far the bar opens from the pointer, down and right (clear of the cursor).
const OFFSET: f32 = 12.0;

/// Where the person is working: each mode has a bar of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    Part,
    Assembly,
    Sketch,
    Drawing,
    Studio(Studio),
}

impl Mode {
    pub(crate) fn of(app: &SolveApp) -> Mode {
        if crate::drawing_workspace::active(app) {
            return Mode::Drawing;
        }
        match Studio::of(app) {
            Studio::Sketch => Mode::Sketch,
            Studio::Part if !app.session.doc.occurrences.is_empty() && app.session.active_component == 0 => Mode::Assembly,
            Studio::Part => Mode::Part,
            other => Mode::Studio(other),
        }
    }

    /// The name the preferences keep this mode's bar under.
    pub(crate) fn key(self) -> &'static str {
        match self {
            Mode::Part => "part",
            Mode::Assembly => "assembly",
            Mode::Sketch => "sketch",
            Mode::Drawing => "drawing",
            Mode::Studio(s) => s.key(),
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Mode::Part => "Part",
            Mode::Assembly => "Assembly",
            Mode::Sketch => "Sketch",
            Mode::Drawing => "Drawing",
            Mode::Studio(s) => s.label(),
        }
    }

    /// The bar as shipped: the gesture ring's tools and a few more for the mode.
    pub(crate) fn defaults(self) -> Vec<String> {
        let own = |v: &[&str]| v.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
        match self {
            Mode::Part => {
                let mut v = own(Studio::Part.defaults());
                v.extend(own(&["solid.fillet", "solid.chamfer", "solid.shell", "ui.smart_edit", "ui.drill_up"]));
                v
            }
            Mode::Sketch => {
                let mut v = own(Studio::Sketch.defaults());
                v.extend(own(&["sketch.arc.center_point", "sketch.trim", "sketch.offset", "ui.ortho", "ui.smart_edit"]));
                v
            }
            Mode::Assembly => own(&[
                "assembly.constraint.create",
                "ui.plane_mate",
                "joint.create",
                "occurrence.move",
                "ui.drill_up",
                "ui.drill_down",
                "ui.smart_edit",
                "inspect.interference",
                "repeat",
                "undo",
            ]),
            // The drawing tabs' tools, in toolbar order.
            Mode::Drawing => solidcraft_engine::command_specs()
                .into_iter()
                .filter(|c| c.tab.starts_with("DRAWING") && c.planned.is_none())
                .map(|c| c.id.to_string())
                .take(PER_ROW)
                .collect(),
            Mode::Studio(s) => own(s.defaults()),
        }
    }
}

/// The bars as the person changed them, by mode: only the changed ones.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub bars: BTreeMap<String, Vec<String>>,
}

impl Config {
    pub(crate) fn tools(&self, mode: Mode) -> Vec<String> {
        match self.bars.get(mode.key()) {
            Some(v) => v.iter().take(MAX_TOOLS).cloned().collect(),
            None => mode.defaults(),
        }
    }
    fn set(&mut self, mode: Mode, tools: Vec<String>) {
        if tools == mode.defaults() {
            self.bars.remove(mode.key());
        } else {
            self.bars.insert(mode.key().to_string(), tools);
        }
    }
}

/// The open bar.
pub(crate) struct Open {
    at: Pos2,
    mode: Mode,
    search: String,
    /// The search box wants the keyboard (once, as it opens).
    focus: bool,
    /// Customizing, with the tool picked to move or remove.
    editing: Option<Option<usize>>,
    /// Where it was drawn last.
    rect: Option<Rect>,
    /// Opened this frame: the press that opened it is not a click elsewhere.
    fresh: bool,
}

/// What the bar keeps between frames.
#[derive(Default)]
pub struct Watch {
    pub(crate) open: Option<Open>,
}

/// Open the bar at the pointer (or the view's middle) for the mode the person is in.
pub fn open(app: &mut SolveApp, at: Option<Pos2>) {
    let at = at.or(app.viewport.mouse).or_else(|| app.viewport.rect.map(|r| r.center())).unwrap_or(Pos2::ZERO);
    app.menu.shortcut.open = Some(Open { at: at + vec2(OFFSET, OFFSET), mode: Mode::of(app), search: String::new(), focus: true, editing: None, rect: None, fresh: true });
}

pub fn is_open(app: &SolveApp) -> bool {
    app.menu.shortcut.open.is_some()
}

/// The bar's tools for `mode` as menu items (unknown ids dropped).
pub(crate) fn items(app: &SolveApp, mode: Mode) -> Vec<(String, Item)> {
    app.preferences.shortcut_bars.tools(mode).into_iter().filter_map(|id| crate::mouse_gesture::item(app, &id).map(|i| (id, i))).collect()
}

/// The commands whose name or id holds every word of `text`, best first (a name that starts
/// with it, then one that holds it).
pub(crate) fn matches(text: &str) -> Vec<(String, String)> {
    let words: Vec<String> = text.split_whitespace().map(str::to_lowercase).collect();
    if words.is_empty() {
        return Vec::new();
    }
    let mut found: Vec<(bool, String, String)> = crate::mouse_gesture::catalog()
        .into_iter()
        .filter(|(id, label)| {
            let (l, i) = (label.to_lowercase(), id.to_lowercase());
            words.iter().all(|w| l.contains(w) || i.contains(w))
        })
        .map(|(id, label)| (!label.to_lowercase().starts_with(&words[0]), id, label))
        .collect();
    found.sort_by(|a, b| a.0.cmp(&b.0));
    found.into_iter().take(MATCHES).map(|(_, id, label)| (id, label)).collect()
}

/// What a frame of the bar asks for.
enum Do {
    Run(Item),
    Add(String),
    Pick(usize),
    Move(usize, isize),
    Remove(usize),
    Reset,
    Edit(bool),
}

/// One frame: draws the open bar and acts on what was clicked.
pub fn show(app: &mut SolveApp, ctx: &egui::Context) {
    let Some(mut bar) = app.menu.shortcut.open.take() else { return };
    let fresh = std::mem::take(&mut bar.fresh);
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        return;
    }
    // A tool or dialog started elsewhere takes over.
    if app.dialog.is_some() && bar.editing.is_none() {
        return;
    }
    let tools = items(app, bar.mode);
    let found = matches(&bar.search);
    let mut act: Option<Do> = None;
    let editing = bar.editing;
    let resp = Container::new(ctx, Surface::Popup)
        .area(egui::Id::unique("sc_shortcut_bar"))
        .fixed_pos(bar.at)
        .constrain(true)
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            for (row, chunk) in tools.chunks(PER_ROW).enumerate() {
                ui.horizontal(|ui| {
                    for (k, (id, item)) in chunk.iter().enumerate() {
                        let i = row * PER_ROW + k;
                        let (rect, _) = ui.allocate_exact_size(vec2(CELL, CELL), Sense::hover());
                        let picked = editing == Some(Some(i));
                        if picked {
                            ui.painter().rect_stroke(rect, 3.0, egui::Stroke::new(2.0, solidcraft_theme::Tokens::get().accent), egui::StrokeKind::Inside);
                        }
                        let on = item.enabled || editing.is_some();
                        let r = ui.add_enabled_ui(on, |ui| crate::ui_kit::icon_button_at(ui, rect, &item.icon, &item.label)).inner;
                        let tip = if item.shortcut.is_empty() { item.label.clone() } else { format!("{} ({})", item.label, item.shortcut) };
                        let r = r.on_hover_text(tip);
                        crate::scenario::publish_handle(&format!("shortcut_bar:{id}"), rect.center());
                        if r.clicked() {
                            act = Some(if editing.is_some() { Do::Pick(i) } else { Do::Run(item.clone()) });
                        }
                    }
                });
            }
            ui.horizontal(|ui| {
                let field = ui.add(egui::TextEdit::singleline(&mut bar.search).hint_text(solidcraft_ui_text::tr_label("Search commands…")).desired_width(CELL * 6.0));
                crate::scenario::publish_handle("shortcut_bar:search", field.rect.center());
                if std::mem::take(&mut bar.focus) {
                    field.request_focus();
                }
                let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if enter && let Some((id, _)) = found.first() {
                    act = Some(found_act(app, id, editing.is_some()));
                }
                let gear = crate::ui_kit::button(ui, if editing.is_some() { "Done" } else { "Customize…" }, crate::ui_kit::Button::Normal);
                crate::scenario::publish_handle("shortcut_bar:customize", gear.rect.center());
                if gear.clicked() {
                    act = Some(Do::Edit(editing.is_none()));
                }
            });
            if let Some(Some(i)) = editing {
                ui.horizontal(|ui| {
                    ui.label(tools.get(i).map(|(_, t)| t.label.clone()).unwrap_or_default());
                    if crate::ui_kit::button(ui, "◀", crate::ui_kit::Button::Enabled(i > 0)).clicked() {
                        act = Some(Do::Move(i, -1));
                    }
                    if crate::ui_kit::button(ui, "▶", crate::ui_kit::Button::Enabled(i + 1 < tools.len())).clicked() {
                        act = Some(Do::Move(i, 1));
                    }
                    let remove = crate::ui_kit::button(ui, "×", crate::ui_kit::Button::Normal);
                    crate::scenario::publish_handle("shortcut_bar:remove", remove.rect.center());
                    if remove.clicked() {
                        act = Some(Do::Remove(i));
                    }
                });
            }
            if editing.is_some() {
                ui.horizontal(|ui| {
                    ui.label(format!("{} bar: pick a tool to move or remove it; search to add one.", bar.mode.label()));
                    if crate::ui_kit::button(ui, "Reset", crate::ui_kit::Button::Normal).clicked() {
                        act = Some(Do::Reset);
                    }
                });
            }
            for (n, (id, label)) in found.iter().enumerate() {
                let r = crate::ui_kit::list_picker(ui, id, label, n == 0);
                crate::scenario::publish_handle(&format!("shortcut_bar:found:{id}"), r.rect.center());
                if r.clicked() {
                    act = Some(found_act(app, id, editing.is_some()));
                }
            }
        });
    bar.rect = Some(resp.response.rect);
    let pointer = ctx.input(|i| i.pointer.latest_pos());
    // A press elsewhere, or the pointer well away while it is only a bar, closes it.
    let outside = ctx.input(|i| i.pointer.any_pressed()) && !resp.response.contains_pointer();
    let away = bar.editing.is_none() && bar.search.is_empty() && crate::quick_menu::left(resp.response.rect, pointer);
    if act.is_none() && !fresh && (outside || away) {
        return;
    }
    let mode = bar.mode;
    let mut list = app.preferences.shortcut_bars.tools(mode);
    match act {
        Some(Do::Run(item)) => {
            let at = bar.at;
            crate::context_menu::run_item(app, &item, at);
            return;
        }
        Some(Do::Add(id)) => {
            if !list.contains(&id) && list.len() < MAX_TOOLS {
                list.push(id);
            }
            bar.search.clear();
        }
        Some(Do::Pick(i)) => bar.editing = Some(Some(i)),
        Some(Do::Move(i, d)) => {
            let j = i.saturating_add_signed(d);
            if j < list.len() && i < list.len() {
                list.swap(i, j);
                bar.editing = Some(Some(j));
            }
        }
        Some(Do::Remove(i)) => {
            if i < list.len() {
                list.remove(i);
            }
            bar.editing = Some(None);
        }
        Some(Do::Reset) => {
            list = mode.defaults();
            bar.editing = Some(None);
        }
        Some(Do::Edit(on)) => bar.editing = on.then_some(None),
        None => {}
    }
    app.preferences.shortcut_bars.set(mode, list);
    app.menu.shortcut.open = Some(bar);
}

/// A search result picked: run it, or (customizing) add it to the bar.
fn found_act(app: &SolveApp, id: &str, editing: bool) -> Do {
    if editing {
        return Do::Add(id.to_string());
    }
    match crate::mouse_gesture::item(app, id) {
        Some(item) => Do::Run(item),
        None => Do::Add(id.to_string()),
    }
}

#[cfg(test)]
#[path = "shortcut_bar_tests.rs"]
mod tests;
