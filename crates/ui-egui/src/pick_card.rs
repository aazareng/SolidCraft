//! The pick card: one pick on a placed part in an assembly opens a small card beside the pointer
//! with where the pick sits and what ties the part down, each a click away.
//!
//! - The trail: the pick, its part, and each sub-assembly around it up to the one placed in the
//!   assembly being edited. A click on a crumb selects it.
//! - Made by: the component pattern that placed the part (a copy, or the part it copies), and the
//!   Mirror feature that made a mirrored face.
//! - Joints: the joints and constraints of the assembly being edited on that top placement (not
//!   those inside sub-assemblies).
//!
//! Each feature and joint row has Edit, Suppress (or Unsuppress) and Delete; a click on a feature's
//! name selects it, on a joint's name edits it. Esc, a click elsewhere, the pointer going well
//! away, or a change of the selection closes the card.

use egui::{Pos2, vec2};
use serde_json::{Value, json};
use solidcraft_engine::Sel;
use solidcraft_engine::doc::FeatureKind;
use solidcraft_engine::doc::joints::{Constraint, RefShape};

use crate::SolveApp;
use crate::dialogs_assembly::picked_snap;
use crate::ui_kit::{Container, Surface};

/// How far from the pointer the card opens, in points.
const OFFSET: f32 = 18.0;
/// The most joints listed (the rest are counted).
const MAX_JOINTS: usize = 12;

/// What the card keeps between frames.
#[derive(Default)]
pub struct Watch {
    seen: Vec<Sel>,
    pub(crate) open: Option<Card>,
    /// The breadcrumbs for the selection they were worked out for.
    pub(crate) crumbs: Option<(Vec<Sel>, Vec<Column>)>,
}

/// A crumb of the trail.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Crumb {
    pub(crate) label: String,
    pub(crate) sel: Sel,
}

/// A feature or joint row.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Row {
    pub(crate) kind: RowKind,
    pub(crate) id: u64,
    pub(crate) label: String,
    pub(crate) icon: String,
    pub(crate) suppressed: bool,
    /// A constraint's rule, and the rules its references also fit: looser ones (left), stricter
    /// ones (right). None for a feature or a joint made as a joint.
    pub(crate) rule: Option<Constraint>,
    pub(crate) looser: Vec<Constraint>,
    pub(crate) stricter: Vec<Constraint>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RowKind {
    Feature,
    Joint,
}

pub(crate) struct Card {
    at: Pos2,
    pub(crate) trail: Vec<Crumb>,
    pub(crate) made_by: Vec<Row>,
    pub(crate) joints: Vec<Row>,
    /// Joints past [`MAX_JOINTS`], not listed.
    more: usize,
    error: Option<String>,
    fresh: bool,
    /// The constraint whose other rules are shown over its icon, to switch to with a click.
    pub(crate) variants: Option<(RowKind, u64)>,
}

/// The placement path of the part a face, edge or point is on (empty: none).
fn part_path(app: &SolveApp, sel: &Sel) -> Vec<u64> {
    if let Sel::Occurrence { path } = sel {
        return path.clone();
    }
    let doc = &app.session.doc;
    let occ = picked_snap(app, sel).and_then(|s| s.param.get("occurrence").and_then(Value::as_u64)).unwrap_or(0);
    let Some(o) = doc.occurrences.iter().find(|o| o.id == occ) else { return Vec::new() };
    let mut path = doc.first_path(o.parent);
    path.push(o.id);
    path
}

fn occ_name(app: &SolveApp, id: u64) -> String {
    app.session.doc.occurrences.iter().find(|o| o.id == id).map(|o| o.name.clone()).unwrap_or_else(|| format!("#{id}"))
}

/// How deep in the path the assembly being edited sits: its placements are the ones at this
/// index.
fn base(app: &SolveApp, path: &[u64]) -> usize {
    match &app.session.active_occurrence_path {
        Some(p) if app.session.active_component != 0 && path.starts_with(p) => p.len(),
        _ => 0,
    }
}

/// Where a pick sits, innermost first: the face, edge or point, the sketch and the feature that
/// made a face, then its part and each sub-assembly around it up to the one placed in the
/// assembly being edited. In a part, the part's own crumbs stop at the feature.
pub(crate) fn trail_for(app: &SolveApp, sel: &Sel) -> Vec<Crumb> {
    let doc = &app.session.doc;
    let mut trail = Vec::new();
    match sel {
        Sel::Face { .. } => trail.push(Crumb { label: "Face".into(), sel: sel.clone() }),
        Sel::Edge { .. } => trail.push(Crumb { label: "Edge".into(), sel: sel.clone() }),
        Sel::Vertex { .. } => trail.push(Crumb { label: "Point".into(), sel: sel.clone() }),
        Sel::Occurrence { .. } => {}
        _ => return trail,
    }
    if let Sel::Face { body, index, point } = sel
        && let Some((made, _)) = crate::smart_edit::face_feature(app, body, *index, *point)
    {
        if let Some(s) = crate::smart_edit::driving_sketch(app, made).filter(|s| *s != made)
            && let Some(f) = doc.feature(s)
        {
            trail.push(Crumb { label: f.name.clone(), sel: Sel::Feature { id: s } });
        }
        if let Some(f) = doc.feature(made) {
            trail.push(Crumb { label: f.name.clone(), sel: Sel::Feature { id: made } });
        }
    }
    let path = part_path(app, sel);
    let from = base(app, &path);
    for end in (from + 1..=path.len()).rev() {
        let id = path[end - 1];
        trail.push(Crumb { label: occ_name(app, id), sel: Sel::Occurrence { path: path[..end].to_vec() } });
    }
    trail
}

/// The card for a selection, when it is one pick on a placed part.
pub(crate) fn card_for(app: &SolveApp, at: Pos2) -> Option<Card> {
    let [sel] = app.session.selection.as_slice() else { return None };
    if !matches!(sel, Sel::Face { .. } | Sel::Edge { .. } | Sel::Vertex { .. } | Sel::Occurrence { .. }) {
        return None;
    }
    let path = part_path(app, sel);
    let from = base(app, &path);
    let top = *path.get(from)?;
    let doc = &app.session.doc;

    let trail = trail_for(app, sel);

    // Made by: the pattern that placed the part (or patterns it), the mirror that made the face.
    let mut made_by = Vec::new();
    let placed: Vec<u64> = path[from..].to_vec();
    let comps: Vec<u64> = placed.iter().filter_map(|o| doc.occurrences.iter().find(|x| x.id == *o).map(|x| x.component)).collect();
    for f in &doc.features {
        let FeatureKind::Pattern { components, occurrences, .. } = &f.kind else { continue };
        if occurrences.iter().any(|o| placed.contains(o)) || components.iter().any(|c| comps.contains(c)) {
            made_by.push(feature_row(f));
        }
    }
    if let Sel::Face { body, index, point } = sel {
        let made = crate::dialogs::blend_feature(&app.session, body, *index).or_else(|| crate::dialogs::feature_of_face(&app.session, body, *index, *point));
        if let Some(f) = made.and_then(|m| doc.feature(m)).filter(|f| matches!(f.kind, FeatureKind::Mirror { .. }))
            && !made_by.iter().any(|r| r.id == f.id)
        {
            made_by.push(feature_row(f));
        }
    }

    // Joints of the assembly being edited on the top placement.
    let mut joints: Vec<Row> = Vec::new();
    for j in &doc.assembly.joints {
        if j.a.occurrence != top && j.b.occurrence != top {
            continue;
        }
        let other = if j.a.occurrence == top { j.b.occurrence } else { j.a.occurrence };
        let other = if other == 0 { "the assembly".to_string() } else { occ_name(app, other) };
        let icon = j.constraint.map_or("joint", crate::dialogs_constrain::rule_icon).to_string();
        let (looser, stricter) = j.constraint.map(|c| variants(c, RefShape::of(&j.a.snap), RefShape::of(&j.b.snap))).unwrap_or_default();
        joints.push(Row { kind: RowKind::Joint, id: j.id, label: format!("{} — {}", j.name, other), icon, suppressed: j.suppressed, rule: j.constraint, looser, stricter });
    }
    let more = joints.len().saturating_sub(MAX_JOINTS);
    joints.truncate(MAX_JOINTS);
    Some(Card { at: at + vec2(OFFSET, OFFSET), trail, made_by, joints, more, error: None, fresh: true, variants: None })
}

fn feature_row(f: &solidcraft_engine::doc::Feature) -> Row {
    let icon = crate::timeline::icon_of(&f.kind).to_string();
    Row { kind: RowKind::Feature, id: f.id, label: f.name.clone(), icon, suppressed: f.suppressed, rule: None, looser: Vec::new(), stricter: Vec::new() }
}

/// Nothing else has the pointer: no tool, dialog, menu, sketch or drag.
fn free(app: &SolveApp) -> bool {
    app.tool.is_none() && app.dialog.is_none() && app.viewport.context_menu.is_none() && app.session.active_sketch.is_none() && !dragging(app)
}

/// A part is being dragged or turned: its moves are not placed yet. (A solve after an edit
/// leaves moves pending too; that is not a drag.)
fn dragging(app: &SolveApp) -> bool {
    crate::joint_drag::active(app) || crate::occurrence_drag::active(app) || crate::occurrence_rotate::active(app)
}

pub fn is_open(app: &SolveApp) -> bool {
    app.menu.card.open.is_some()
}

/// One column of the breadcrumbs: a crumb on the trail, what sits above it (the mates of a
/// part), and what hangs below it (a feature's driving sketch, a part's planes and origin).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Column {
    pub(crate) above: Vec<Mark>,
    pub(crate) main: Cell,
    pub(crate) below: Vec<Cell>,
}

/// A crumb or an item under one: its name, what a click selects (none: nothing to select), and
/// its icon.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Cell {
    pub(crate) label: String,
    pub(crate) sel: Option<Sel>,
    pub(crate) icon: &'static str,
}

/// A mate over a part: a click edits it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Mark {
    pub(crate) joint: u64,
    pub(crate) name: String,
    pub(crate) icon: String,
}

/// The most mates shown over one part.
const MAX_MARKS: usize = 6;

/// A column with only its crumb.
fn col(label: String, sel: Option<Sel>, icon: &'static str) -> Column {
    Column { above: Vec::new(), main: Cell { label, sel, icon }, below: Vec::new() }
}

/// A feature's column, its driving sketch under it.
fn feature_col(app: &SolveApp, id: u64) -> Option<Column> {
    let doc = &app.session.doc;
    let f = doc.feature(id)?;
    let mut c = col(f.name.clone(), Some(Sel::Feature { id }), crate::timeline::icon_of(&f.kind));
    if let Some(s) = crate::smart_edit::driving_sketch(app, id).filter(|s| *s != id)
        && let Some(sk) = doc.feature(s)
    {
        c.below.push(Cell { label: sk.name.clone(), sel: Some(Sel::Feature { id: s }), icon: "sketch" });
    }
    Some(c)
}

/// A placement's (or, for an empty path, the design's) planes and origin, as the Browser's
/// Origin folder names them.
fn origin_cells(app: &SolveApp, path: &[u64]) -> Vec<Cell> {
    let names = &app.preferences.origin_plane_names;
    ["XY", "XZ", "YZ", "O"]
        .into_iter()
        .map(|k| {
            let name = if path.is_empty() { k.to_string() } else { solidcraft_engine::origins::key(path, k) };
            let (sel, icon) = match k {
                "O" if path.is_empty() => (Sel::Vertex { body: String::new(), point: solidcore_record::Vec3::ZERO }, "point"),
                "O" => (Sel::ConstructionPoint { name }, "point"),
                _ => (Sel::Plane { name }, "plane"),
            };
            let label = if k == "O" { "Origin".to_string() } else { names.label(k).to_string() };
            Cell { label, sel: Some(sel), icon }
        })
        .collect()
}

/// The breadcrumbs of a pick, from it outward: the face (edge, point), the feature that made it
/// with its driving sketch under it, the body, the part with its planes and origin under it and
/// its mates over it, then each assembly around it the same way; in a part, the design last.
pub(crate) fn hierarchy(app: &SolveApp, sel: &Sel) -> Vec<Column> {
    let doc = &app.session.doc;
    let mut out = Vec::new();
    // The pick itself.
    let body = match sel {
        Sel::Face { body, .. } => {
            out.push(col("Face".into(), Some(sel.clone()), "offset_face"));
            Some(body.clone())
        }
        Sel::Edge { body, .. } => {
            out.push(col("Edge".into(), Some(sel.clone()), "axis_edge"));
            Some(body.clone())
        }
        Sel::Vertex { body, .. } => {
            out.push(col("Point".into(), Some(sel.clone()), "point"));
            Some(body.clone())
        }
        Sel::Body { name } => Some(name.clone()),
        Sel::Feature { id } => {
            out.extend(feature_col(app, *id));
            None
        }
        Sel::Occurrence { .. } => None,
        _ => return out,
    };
    // The feature that made the face, and its sketch under it.
    if let Sel::Face { body, index, point } = sel
        && let Some((made, _)) = crate::smart_edit::face_feature(app, body, *index, *point)
    {
        out.extend(feature_col(app, made));
    }
    // The body.
    if let Some(b) = body.as_ref().filter(|b| !b.is_empty()) {
        out.push(col(b.clone(), Some(Sel::Body { name: b.clone() }), "body"));
    }
    // The part and each assembly around it, up to the one being edited.
    let path = match sel {
        Sel::Occurrence { path } => path.clone(),
        Sel::Body { name } => {
            let component = app.session.world_state().ok().and_then(|st| st.body(name).map(|b| b.component)).unwrap_or(0);
            if component == 0 { Vec::new() } else { doc.first_path(component) }
        }
        _ => part_path(app, sel),
    };
    let from = base(app, &path);
    for end in (from + 1..=path.len()).rev() {
        let id = path[end - 1];
        let mut c = col(occ_name(app, id), Some(Sel::Occurrence { path: path[..end].to_vec() }), "component");
        c.below = origin_cells(app, &path[..end]);
        c.above = doc
            .assembly
            .joints
            .iter()
            .filter(|j| j.a.occurrence == id || j.b.occurrence == id)
            .take(MAX_MARKS)
            .map(|j| Mark { joint: j.id, name: j.name.clone(), icon: j.constraint.map_or("joint", crate::dialogs_constrain::rule_icon).to_string() })
            .collect();
        out.push(c);
    }
    // In a part (nothing placed), the design last, with its own planes.
    if path.is_empty() && app.session.active_component == 0 {
        let name = if doc.name.is_empty() { "Design".to_string() } else { doc.name.clone() };
        let mut c = col(name, None, "component");
        c.below = origin_cells(app, &[]);
        out.push(c);
    }
    out
}

/// What a click on the breadcrumbs asks for.
enum CrumbDo {
    Select(Sel),
    EditJoint(u64),
}

/// A crumb or an item under one: its icon and name, bold for the pick.
fn crumb_cell(ui: &mut egui::Ui, key: &str, cell: &Cell, lit: bool) -> egui::Response {
    let r = ui
        .push_id(key, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 3.0;
                let (rect, _) = ui.allocate_exact_size(vec2(14.0, 14.0), egui::Sense::hover());
                crate::ui_kit::icons::paint_icon(ui.painter(), rect, cell.icon, if lit { 1.0 } else { 0.75 });
                let text = if lit { egui::RichText::new(&cell.label).strong() } else { egui::RichText::new(&cell.label) };
                ui.add(egui::Label::new(text).sense(egui::Sense::click()))
            })
            .inner
        })
        .inner;
    crate::scenario::publish_handle(&format!("breadcrumb:{key}"), r.rect.center());
    r
}

/// The breadcrumbs: while one face, edge, point, body, feature or part is picked, its whole
/// hierarchy in a small grid laid over the view (below the ribbon, right of the Browser), the
/// pick on the left. A click selects what it names; a mate's mark opens it to edit. A faint grip
/// moves it (double-click: back in place).
fn breadcrumbs(app: &mut SolveApp, ctx: &egui::Context) {
    if !app.preferences.pick_card || app.session.active_sketch.is_some() || app.session.selection.len() != 1 {
        app.menu.card.crumbs = None;
        return;
    }
    let sel = app.session.selection.clone();
    // Mid-drag the parts are being moved on the GPU: working the trail out would place them all
    // again every frame. It is worked out once the drag ends.
    if dragging(app) {
        return;
    }
    if app.menu.card.crumbs.as_ref().is_none_or(|(seen, _)| *seen != sel) {
        let columns = sel.first().map(|s| hierarchy(app, s)).unwrap_or_default();
        app.menu.card.crumbs = Some((sel, columns));
    }
    let Some((_, columns)) = app.menu.card.crumbs.clone() else { return };
    let Some(view) = app.viewport.layout.as_ref().map(|l| l.content).or(app.viewport.rect) else { return };
    if columns.is_empty() {
        return;
    }
    // Where it sits unless moved: the view's top left corner the panels leave clear, right of
    // the Browser and below the ribbon (which may float over the view).
    let left = app.viewport.layout.as_ref().map_or(view.left(), |l| l.middle.left().max(view.left()));
    let top = view.top().max(app.ui.ribbon_bottom);
    let auto = egui::pos2(left + 12.0, top + 8.0);
    let screen = ctx.content_rect();
    let at = app.preferences.breadcrumbs_at.map(|[x, y]| egui::pos2(x, y)).filter(|p| screen.contains(*p)).unwrap_or(auto);
    let mut act: Option<CrumbDo> = None;
    let mut moved: Option<Option<[f32; 2]>> = None;
    Container::new(ctx, Surface::Popup)
        .area(egui::Id::unique("sc_breadcrumbs"))
        // Under menus and popups (a layer below theirs), so an open menu covers it.
        .fixed_pos(at)
        .order(egui::Order::Middle)
        .show(ctx, |ui| {
            egui::Grid::new("sc_breadcrumbs_grid").spacing(vec2(6.0, 2.0)).show(ui, |ui| {
                // Over the crumbs: the mates of each part.
                ui.label("");
                for (i, c) in columns.iter().enumerate() {
                    if i > 0 {
                        ui.label("");
                    }
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 1.0;
                        for m in &c.above {
                            let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), egui::Sense::hover());
                            let b = ui.push_id(("mate", m.joint), |ui| crate::ui_kit::icon_button_at(ui, r, &m.icon, &m.name)).inner.on_hover_text(&m.name);
                            crate::scenario::publish_handle(&format!("breadcrumb:mate:{}", m.joint), r.center());
                            if b.clicked() {
                                act = Some(CrumbDo::EditJoint(m.joint));
                            }
                        }
                    });
                }
                ui.end_row();
                // The trail, the pick first; a faint grip moves it.
                let (grip, resp) = ui.allocate_exact_size(vec2(8.0, 16.0), egui::Sense::click_and_drag());
                let ink = if resp.hovered() || resp.dragged() { ui.visuals().text_color() } else { ui.visuals().weak_text_color().gamma_multiply(0.6) };
                for row in 0..3 {
                    for dot in 0..2 {
                        ui.painter().circle_filled(grip.left_top() + vec2(2.0 + 4.0 * dot as f32, 4.0 + 4.0 * row as f32), 1.0, ink);
                    }
                }
                let resp = resp.on_hover_text("Drag to move; double-click to put it back");
                crate::scenario::publish_handle("breadcrumb:grip", grip.center());
                if resp.double_clicked() {
                    moved = Some(None);
                } else if resp.dragged() {
                    let p = at + resp.drag_delta();
                    moved = Some(Some([p.x, p.y]));
                }
                for (i, c) in columns.iter().enumerate() {
                    if i > 0 {
                        ui.label(egui::RichText::new("›").weak());
                    }
                    let r = crumb_cell(ui, &i.to_string(), &c.main, i == 0);
                    if r.clicked()
                        && i > 0
                        && let Some(s) = &c.main.sel
                    {
                        act = Some(CrumbDo::Select(s.clone()));
                    }
                }
                ui.end_row();
                // Under the crumbs: a feature's sketch, a part's planes and origin.
                ui.label("");
                for (i, c) in columns.iter().enumerate() {
                    if i > 0 {
                        ui.label("");
                    }
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        for (k, cell) in c.below.iter().enumerate() {
                            let r = crumb_cell(ui, &format!("{i}:{k}"), cell, false);
                            if r.clicked()
                                && let Some(s) = &cell.sel
                            {
                                act = Some(CrumbDo::Select(s.clone()));
                            }
                        }
                    });
                }
                ui.end_row();
            });
        });
    if let Some(at) = moved {
        app.preferences.breadcrumbs_at = at;
    }
    match act {
        Some(CrumbDo::Select(s)) => app.session.selection = vec![s],
        Some(CrumbDo::EditJoint(id)) => {
            if let Some(d) = crate::dialogs_assembly::open_joint(app, id) {
                app.tool = None;
                app.dialog = Some(d);
            }
        }
        None => {}
    }
}


/// What a row's button asks for.
enum Do {
    Select(Sel),
    Edit(RowKind, u64),
    Suppress(RowKind, u64, bool),
    Delete(RowKind, u64),
    /// Show (or hide) the other rules a constraint can be.
    Variants((RowKind, u64)),
    /// Make the constraint that rule instead.
    Swap(u64, Constraint),
}

/// How much a rule holds: 1 a direction (parallel, square, at an angle), 2 a direction and a
/// distance or a line (distance, concentric, tangent, on an axis), 3 the two together (mate,
/// flush, insert).
fn strictness(c: Constraint) -> u8 {
    match c {
        Constraint::Parallel | Constraint::Perpendicular | Constraint::Angle => 1,
        Constraint::Distance | Constraint::Concentric | Constraint::Tangent | Constraint::PointOnAxis => 2,
        Constraint::Mate | Constraint::Flush | Constraint::Insert => 3,
    }
}

/// The other rules references of shapes `a` and `b` fit, beside rule `c`: those that hold as much
/// or less (nearest first, to its left), and those that hold more (nearest first, to its right).
pub(crate) fn variants(c: Constraint, a: RefShape, b: RefShape) -> (Vec<Constraint>, Vec<Constraint>) {
    let s = strictness(c);
    let all: Vec<Constraint> = Constraint::fitting(a, b).into_iter().filter(|x| *x != c).collect();
    let mut looser: Vec<Constraint> = all.iter().copied().filter(|x| strictness(*x) <= s).collect();
    looser.sort_by_key(|x| std::cmp::Reverse(strictness(*x)));
    let mut stricter: Vec<Constraint> = all.into_iter().filter(|x| strictness(*x) > s).collect();
    stricter.sort_by_key(|x| strictness(*x));
    (looser, stricter)
}

/// One frame: opens the card when the selection becomes one pick on a placed part, closes it
/// when that changes, and draws it.
pub fn show(app: &mut SolveApp, ctx: &egui::Context) {
    breadcrumbs(app, ctx);
    if app.session.selection != app.menu.card.seen {
        app.menu.card.seen = app.session.selection.clone();
        app.menu.card.open = None;
        let at = ctx.input(|i| i.pointer.latest_pos()).filter(|_| app.viewport.mouse.is_some());
        if let Some(at) = at
            && free(app)
            && app.preferences.pick_card
        {
            app.menu.card.open = card_for(app, at);
        }
    }
    let Some(mut card) = app.menu.card.open.take() else { return };
    let fresh = std::mem::take(&mut card.fresh);
    if !free(app) || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        return;
    }
    let mut act: Option<Do> = None;
    let mut icons: Vec<((RowKind, u64), egui::Rect)> = Vec::new();
    let resp = Container::new(ctx, Surface::Popup)
        .area(egui::Id::unique("sc_pick_card"))
        .fixed_pos(card.at)
        .constrain(true)
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            // The trail, outermost on the left.
            ui.horizontal(|ui| {
                for (n, crumb) in card.trail.iter().rev().enumerate() {
                    if n > 0 {
                        ui.label("›");
                    }
                    let r = ui.link(&crumb.label);
                    crate::scenario::publish_handle(&format!("pick_card:crumb:{n}"), r.rect.center());
                    if r.clicked() {
                        act = Some(Do::Select(crumb.sel.clone()));
                    }
                }
            });
            if !card.made_by.is_empty() {
                ui.separator();
                ui.label(solidcraft_ui_text::tr_label("Made by"));
                for row in &card.made_by {
                    rows(ui, row, &mut act, &mut icons);
                }
            }
            ui.separator();
            if card.joints.is_empty() {
                ui.label(solidcraft_ui_text::tr_label("No joints"));
            } else {
                for row in &card.joints {
                    rows(ui, row, &mut act, &mut icons);
                }
                if card.more > 0 {
                    ui.label(format!("and {} more", card.more));
                }
            }
            if let Some(e) = &card.error {
                crate::ui_kit::status_line(ui, e, crate::ui_kit::StatusTone::Error);
            }
        });
    // The cross over the icon clicked: the other rules a constraint can be to its left (looser)
    // and right (stricter), Edit above it, Suppress and Delete below.
    let mut over = resp.response.rect;
    if let Some((kind, id)) = card.variants
        && let Some(row) = card.joints.iter().chain(card.made_by.iter()).find(|r| r.kind == kind && r.id == id).cloned()
        && let Some((_, icon)) = icons.iter().find(|(k, _)| *k == (kind, id))
    {
        let key = row_key(&row);
        let left_n = row.looser.len();
        // The popup's own margin, so the middle cell lands on the icon.
        let margin = 6.0;
        let origin = egui::pos2(icon.center().x - CROSS * (left_n as f32 + 0.5) - margin, icon.center().y - CROSS * 1.5 - margin);
        let mut clicked: Option<Do> = None;
        let cross = Container::new(ctx, Surface::Popup)
            .area(egui::Id::unique("sc_pick_card_cross"))
            .fixed_pos(origin)
            .order(egui::Order::Tooltip)
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                let blank = |ui: &mut egui::Ui, n: usize| {
                    for _ in 0..n {
                        ui.allocate_exact_size(vec2(CROSS, CROSS), egui::Sense::hover());
                    }
                };
                // Edit, above.
                ui.horizontal(|ui| {
                    blank(ui, left_n);
                    if cross_cell(ui, &format!("{key}:edit"), "edit", "Edit", false) {
                        clicked = Some(Do::Edit(row.kind, row.id));
                    }
                });
                // The rules: looser, this one, stricter.
                ui.horizontal(|ui| {
                    for c in row.looser.iter().rev().copied() {
                        let label = crate::dialogs_constrain::label(c);
                        if cross_cell(ui, &format!("variant:{}", c.name()), crate::dialogs_constrain::rule_icon(c), &format!("Make it {label}"), false) {
                            clicked = Some(Do::Swap(row.id, c));
                        }
                    }
                    let tip = row.rule.map_or_else(|| row.label.clone(), |r| crate::dialogs_constrain::label(r).to_string());
                    let here: String = row.rule.map_or_else(|| row.icon.clone(), |r| crate::dialogs_constrain::rule_icon(r).to_string());
                    let at = row.rule.map_or_else(|| format!("{key}:here"), |r| format!("variant:{}", r.name()));
                    if cross_cell(ui, &at, &here, &tip, true) {
                        clicked = Some(Do::Variants((row.kind, row.id)));
                    }
                    for c in row.stricter.iter().copied() {
                        let label = crate::dialogs_constrain::label(c);
                        if cross_cell(ui, &format!("variant:{}", c.name()), crate::dialogs_constrain::rule_icon(c), &format!("Make it {label}"), false) {
                            clicked = Some(Do::Swap(row.id, c));
                        }
                    }
                });
                // Suppress, then Delete, below.
                ui.horizontal(|ui| {
                    blank(ui, left_n);
                    let label = if row.suppressed { "Unsuppress" } else { "Suppress" };
                    if cross_cell(ui, &format!("{key}:suppress"), "suppress", label, false) {
                        clicked = Some(Do::Suppress(row.kind, row.id, !row.suppressed));
                    }
                });
                ui.horizontal(|ui| {
                    blank(ui, left_n);
                    if cross_cell(ui, &format!("{key}:delete"), "delete", "Delete", false) {
                        clicked = Some(Do::Delete(row.kind, row.id));
                    }
                });
            });
        if clicked.is_some() {
            act = clicked;
        }
        over = over.union(cross.response.rect);
    }
    let pointer = ctx.input(|i| i.pointer.latest_pos());
    let outside = ctx.input(|i| i.pointer.any_pressed()) && !pointer.is_some_and(|p| over.contains(p));
    let away = crate::quick_menu::left(over, pointer);
    if act.is_none() && !fresh && (outside || away) {
        return;
    }
    let Some(act) = act else {
        app.menu.card.open = Some(card);
        return;
    };
    // The cross opens and closes without leaving the card.
    if let Do::Variants(k) = act {
        card.variants = if card.variants == Some(k) { None } else { Some(k) };
        app.menu.card.open = Some(card);
        return;
    }
    let refresh = matches!(act, Do::Suppress(..) | Do::Delete(..));
    let swapped = if let Do::Swap(id, rule) = act { Some((id, rule)) } else { None };
    match run(app, act) {
        // A suppress or delete keeps the card, brought up to date.
        Ok(()) if refresh => {
            if let Some(mut again) = card_for(app, card.at - vec2(OFFSET, OFFSET)) {
                again.fresh = false;
                app.menu.card.open = Some(again);
            }
        }
        // A switched mate is offered as a new one is: Flip, its value and Delete by the pointer.
        Ok(()) if swapped.is_some() => {
            if let Some((id, rule)) = swapped
                && let Some(j) = app.session.doc.assembly.joints.iter().find(|j| j.id == id)
            {
                let value = match rule {
                    Constraint::Angle => j.exprs.angle.clone().unwrap_or_else(|| format!("{:.3} deg", j.angle.to_degrees())),
                    _ => j.exprs.offset.clone().unwrap_or_else(|| format!("{} mm", (j.offset * 1000.0).round() / 1000.0)),
                };
                let name = j.name.clone();
                let at = pointer.unwrap_or(card.at);
                crate::quick_constrain::offer_made(app, name, rule, value, at);
            }
        }
        Ok(()) => {}
        Err(e) => {
            card.error = Some(e);
            app.menu.card.open = Some(card);
        }
    }
}

/// One square of the cross: an icon button, outlined when it is the row's own.
fn cross_cell(ui: &mut egui::Ui, key: &str, icon: &str, tip: &str, here: bool) -> bool {
    let (r, _) = ui.allocate_exact_size(vec2(CROSS, CROSS), egui::Sense::hover());
    let t = solidcraft_theme::Tokens::get();
    if here {
        ui.painter().rect_stroke(r.shrink(1.0), 3.0, egui::Stroke::new(1.5, t.accent), egui::StrokeKind::Inside);
    }
    let b = ui.push_id(key, |ui| crate::ui_kit::icon_button_colored(ui, r.shrink(2.0), icon, tip, Some(t.form_ink))).inner.on_hover_text(tip);
    crate::scenario::publish_handle(&format!("pick_card:{key}"), r.center());
    b.clicked()
}

/// One square of the cross, in points.
const CROSS: f32 = 26.0;

/// A row's handle key: `joint:<id>` or `feature:<id>`.
fn row_key(row: &Row) -> String {
    match row.kind {
        RowKind::Feature => format!("feature:{}", row.id),
        RowKind::Joint => format!("joint:{}", row.id),
    }
}

/// A feature or joint row: its icon, which opens the cross of what can be done with it, and its
/// name (a click selects a feature, edits a joint).
fn rows(ui: &mut egui::Ui, row: &Row, act: &mut Option<Do>, icons: &mut Vec<((RowKind, u64), egui::Rect)>) {
    let key = row_key(row);
    ui.push_id(&key, |ui| {
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(22.0, 22.0), egui::Sense::hover());
            let ink = solidcraft_theme::Tokens::get().form_ink;
            let tip = if row.rule.is_some() { "Change, edit, suppress or delete" } else { "Edit, suppress or delete" };
            let b = crate::ui_kit::icon_button_colored(ui, rect, &row.icon, tip, Some(ink)).on_hover_text(tip);
            crate::scenario::publish_handle(&format!("pick_card:{key}:rule"), rect.center());
            if b.clicked() {
                *act = Some(Do::Variants((row.kind, row.id)));
            }
            icons.push(((row.kind, row.id), rect));
            let text = if row.suppressed { egui::RichText::new(&row.label).weak().strikethrough() } else { egui::RichText::new(&row.label) };
            let name = ui.add(egui::Label::new(text).truncate().sense(egui::Sense::click())).on_hover_text(&row.label);
            crate::scenario::publish_handle(&format!("pick_card:{key}"), name.rect.center());
            if name.clicked() {
                *act = Some(match row.kind {
                    RowKind::Feature => Do::Select(Sel::Feature { id: row.id }),
                    RowKind::Joint => Do::Edit(row.kind, row.id),
                });
            }
        });
    });
}

fn run(app: &mut SolveApp, act: Do) -> Result<(), String> {
    match act {
        Do::Select(sel) => {
            app.session.selection = vec![sel];
            Ok(())
        }
        Do::Edit(RowKind::Feature, id) => {
            app.edit_feature(id);
            Ok(())
        }
        Do::Edit(RowKind::Joint, id) => {
            if let Some(d) = crate::dialogs_assembly::open_joint(app, id) {
                app.tool = None;
                app.dialog = Some(d);
            }
            Ok(())
        }
        Do::Suppress(RowKind::Feature, id, on) => app.run("timeline.suppress", json!({"feature": id, "suppressed": on})).map(|_| ()),
        Do::Suppress(RowKind::Joint, id, on) => app.run("joint.edit", json!({"joint": id, "suppressed": on})).map(|_| ()),
        Do::Delete(RowKind::Feature, id) => app.run("timeline.delete", json!({"feature": id})).map(|_| ()),
        Do::Delete(RowKind::Joint, id) => app.run("joint.delete", json!({"joint": id})).map(|_| ()),
        // The same references kept, the rule changed in place (its gap carries over).
        Do::Swap(id, rule) => {
            let j = app.session.doc.assembly.joints.iter().find(|j| j.id == id).cloned().ok_or("the constraint is gone")?;
            let mut p = json!({"constraint": j.name, "type": rule.name()});
            if rule == Constraint::Angle {
                // The angle the two references make now, so nothing turns.
                let st = app.session.model.state();
                let angle = match (app.session.doc.origin_world(&st, &j.a), app.session.doc.origin_world(&st, &j.b)) {
                    (Ok(a), Ok(b)) => (0..3).map(|i| a[2][i] * b[2][i]).sum::<f64>().clamp(-1.0, 1.0).acos().to_degrees(),
                    _ => 90.0,
                };
                p["angle"] = json!(format!("{:.3} deg", if angle.is_finite() { angle } else { 90.0 }));
            }
            app.run("assembly.constraint.edit", p).map(|_| ())
        }
        Do::Variants(_) => Ok(()),
    }
}

#[cfg(test)]
#[path = "pick_card_tests.rs"]
mod tests;
