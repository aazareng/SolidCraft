//! The quick constrain bar: when the selection becomes references on different parts that a rule
//! fits, and nothing else has the pointer, a row of those rules opens beside the pointer, the
//! likeliest first and marked as the default.
//!
//! - Two picks on two parts (two faces, a face and a round edge, two holes…): the assembly
//!   constraints they fit (`Constraint::fitting`). A click on a rule that needs no value adds it at
//!   once (`assembly.constraint.create`, the first pick held, the second moved, as the Constrain
//!   dialog does); Distance and Angle show a value, filled with how far apart or how turned the two
//!   are now, that Enter or Apply adds. Flip turns the moving part over.
//! - Four flat faces, two on each of two parts: Width, the one part centred between the other's
//!   two faces. Made of what the design already has: a Midplane on each part, and a mate (or flush,
//!   as they face now) between the two midplanes.
//! - Three: a flat face on each of two parts and a plane (a datum plane when one is picked, else
//!   the last pick): Symmetric, the one face mirrored onto the other about the plane. It needs the
//!   plane and one of the faces not to move against each other (on one part, or both fixed), and
//!   that face square to the plane's normal: a mirror plane (an Offset Plane of that face, twice
//!   its distance to the plane back through it) on its part, and a mate of the other face on it.
//!
//! Each rule is one undo step. Esc, a click elsewhere or a change of the selection closes the bar.

use egui::{Pos2, vec2};
use serde_json::{Value, json};
use solidcraft_engine::Sel;
use solidcraft_engine::doc::joints::{Constraint, RefShape};
use solidcraft_engine::geom::prelude::*;

use crate::SolveApp;
use crate::dialogs_assembly::{SnapKind, SnapPt, picked_snap};
use crate::ui_kit::{Container, Surface};

/// How far from the pointer the bar opens, in points (down and right, clear of the cursor).
const OFFSET: f32 = 18.0;
/// How near to parallel two normals must be (the cosine of the angle between them).
const PARALLEL: f64 = 0.999;

/// What the bar keeps between frames.
#[derive(Default)]
pub struct Watch {
    /// The selection last seen: a different one opens or closes the bar.
    seen: Vec<Sel>,
    /// The bar, while it is open.
    open: Option<Bar>,
}

/// A pick as the rules see it: its snap and the part (occurrence, 0 for the design) it is on.
#[derive(Clone)]
struct Pick {
    snap: SnapPt,
    part: u64,
}

/// What the picks make.
enum Picks {
    /// Two picks on two parts, in the order picked (the first held, the second moved).
    Pair { params: [Value; 2] },
    /// Two flat faces on each of two parts: the first part's, then the second's.
    Width { a: [Pick; 2], b: [Pick; 2] },
    /// The face that stays (on the part the plane is on), the face that moves, and the plane.
    Symmetric { held: Pick, moved: Pick, plane: Pick },
}

struct Bar {
    /// Where it opened.
    at: Pos2,
    picks: Picks,
    /// The rules they fit, the likeliest first (one, for Width and Symmetric).
    rules: Vec<Rule>,
    /// The rule chosen for a value (Distance, Angle), awaiting Enter.
    chosen: Option<Rule>,
    /// Distance's gap and Angle's angle, as typed.
    offset: String,
    angle: String,
    flip: bool,
    /// The value field wants the keyboard (once, when its rule is chosen).
    focus: bool,
    error: Option<String>,
    /// The constraint the bar just made, which it now offers to flip, reverse or delete.
    made: Option<Made>,
    /// Where the bar was drawn last (faded while the pointer is off it).
    rect: Option<egui::Rect>,
}

/// A constraint the bar made, still open to change.
struct Made {
    /// Its name, as `assembly.constraint.edit` takes it.
    name: String,
    rule: Rule,
    /// Distance's gap or Angle's angle, as typed.
    value: String,
}

/// How faded the bar is while the pointer is off it, once it has made a constraint.
const FADED: f32 = 0.45;

/// A rule on the bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Rule {
    Constraint(Constraint),
    Width,
    Symmetric,
}

impl Rule {
    fn label(self) -> &'static str {
        match self {
            Rule::Constraint(c) => crate::dialogs_constrain::label(c),
            Rule::Width => "Width",
            Rule::Symmetric => "Symmetric",
        }
    }
    fn icon(self) -> &'static str {
        match self {
            Rule::Constraint(c) => crate::dialogs_constrain::rule_icon(c),
            Rule::Width => "plane_midplane",
            Rule::Symmetric => "c_symmetry",
        }
    }
    /// The rule asks for a value before it is added.
    fn takes_value(self) -> bool {
        matches!(self, Rule::Constraint(Constraint::Distance | Constraint::Angle))
    }
}

/// The rules the picks `a` and `b` fit, the likeliest first: as the Constrain dialog orders them,
/// except that two faces looking the same way are flush first rather than mated.
fn ranked(a: &SnapPt, b: &SnapPt, sa: RefShape, sb: RefShape) -> Vec<Constraint> {
    let mut rules = Constraint::fitting(sa, sb);
    if sa == RefShape::Plane
        && sb == RefShape::Plane
        && a.z.dot(b.z) > PARALLEL
        && let (Some(m), Some(f)) = (rules.iter().position(|c| *c == Constraint::Mate), rules.iter().position(|c| *c == Constraint::Flush))
    {
        rules.swap(m, f);
    }
    rules
}

/// Two decimals at most, with no trailing zeros.
fn short(v: f64) -> String {
    let s = format!("{v:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn pick(app: &SolveApp, sel: &Sel) -> Option<Pick> {
    let snap = picked_snap(app, sel)?;
    let part = snap.param.get("occurrence").and_then(Value::as_u64).unwrap_or(0);
    Some(Pick { snap, part })
}

/// A flat face or a datum plane.
fn flat(p: &Pick) -> bool {
    p.snap.kind == SnapKind::FaceCenter
}

/// A flat face of a body (not a datum plane): what a Midplane or an Offset Plane is made from.
fn flat_face(p: &Pick) -> bool {
    flat(p) && p.snap.param.get("face").is_some()
}

/// A part that never moves: the design itself, or a grounded placement.
fn fixed(app: &SolveApp, part: u64) -> bool {
    part == 0 || app.session.doc.occurrences.iter().any(|o| o.id == part && o.grounded)
}

/// Two flat faces on each of two parts: Width.
fn width(picks: &[Pick]) -> Option<Picks> {
    if picks.len() != 4 || !picks.iter().all(flat_face) {
        return None;
    }
    let first = picks[0].part;
    let (a, b): (Vec<&Pick>, Vec<&Pick>) = picks.iter().partition(|p| p.part == first);
    let [a0, a1] = a.as_slice() else { return None };
    let [b0, b1] = b.as_slice() else { return None };
    (b0.part == b1.part).then(|| Picks::Width { a: [(*a0).clone(), (*a1).clone()], b: [(*b0).clone(), (*b1).clone()] })
}

/// A face on each of two parts and a plane the first or second face keeps still against: Symmetric.
fn symmetric(app: &SolveApp, sels: &[Sel], picks: &[Pick]) -> Option<Picks> {
    if picks.len() != 3 || !picks.iter().all(flat) {
        return None;
    }
    // The plane: the one datum plane picked, else the last pick.
    let datums: Vec<usize> = (0..3).filter(|i| matches!(sels[*i], Sel::Plane { .. })).collect();
    let k = if let [only] = datums.as_slice() { *only } else { 2 };
    let plane = picks[k].clone();
    let faces: Vec<&Pick> = picks.iter().enumerate().filter(|(i, _)| *i != k).map(|(_, p)| p).collect();
    let [f0, f1] = faces.as_slice() else { return None };
    if f0.part == f1.part || !flat_face(f0) || !flat_face(f1) {
        return None;
    }
    // The face that holds still against the plane, and square to it (its mirror is then a plane
    // parallel to it), stays; the other moves.
    let still = |f: &Pick| (f.part == plane.part || (fixed(app, f.part) && fixed(app, plane.part))) && f.snap.z.dot(plane.snap.z).abs() > PARALLEL;
    let (held, moved) = if still(f0) {
        (*f0, *f1)
    } else if still(f1) {
        (*f1, *f0)
    } else {
        return None;
    };
    Some(Picks::Symmetric { held: held.clone(), moved: moved.clone(), plane })
}

/// The bar for the selection, when its picks make a rule.
fn bar_for(app: &SolveApp, at: Pos2) -> Option<Bar> {
    let sels = app.session.selection.clone();
    if !(2..=4).contains(&sels.len()) {
        return None;
    }
    let picks: Vec<Pick> = sels.iter().map(|s| pick(app, s)).collect::<Option<_>>()?;
    let mut bar = Bar {
        at: at + vec2(OFFSET, OFFSET),
        picks: Picks::Pair { params: [Value::Null, Value::Null] },
        rules: Vec::new(),
        chosen: None,
        offset: "0 mm".into(),
        angle: "90 deg".into(),
        flip: false,
        focus: false,
        error: None,
        made: None,
        rect: None,
    };
    match picks.as_slice() {
        [a, b] => {
            if a.part == b.part {
                return None;
            }
            let (sa, sb) = (crate::dialogs_constrain::shape(&a.snap), crate::dialogs_constrain::shape(&b.snap));
            bar.rules = ranked(&a.snap, &b.snap, sa, sb).into_iter().map(Rule::Constraint).collect();
            // The values the two have now: the gap along the first's normal, the angle between them.
            let gap = (b.snap.at - a.snap.at).dot(a.snap.z).abs();
            let turned = a.snap.z.dot(b.snap.z).clamp(-1.0, 1.0).acos().to_degrees();
            bar.offset = format!("{} mm", short(if gap.is_finite() { gap } else { 0.0 }));
            bar.angle = format!("{} deg", short(if turned.is_finite() { turned } else { 90.0 }));
            bar.picks = Picks::Pair { params: [a.snap.param.clone(), b.snap.param.clone()] };
        }
        [_, _, _] => {
            bar.picks = symmetric(app, &sels, &picks)?;
            bar.rules = vec![Rule::Symmetric];
        }
        _ => {
            bar.picks = width(&picks)?;
            bar.rules = vec![Rule::Width];
        }
    }
    (!bar.rules.is_empty()).then_some(bar)
}

/// Nothing else has the pointer: no tool, dialog, menu or sketch.
fn free(app: &SolveApp) -> bool {
    app.tool.is_none() && app.dialog.is_none() && app.viewport.context_menu.is_none() && crate::sketch_view::active_sketch(app).is_none()
}

/// Open the bar on a constraint made (or changed) elsewhere, as it is after the bar made one: its
/// name, Flip, its value and Reverse (Distance, Angle) and Delete, faded beside the pointer `at`.
pub(crate) fn offer_made(app: &mut SolveApp, name: String, rule: Constraint, value: String, at: Pos2) {
    let rule = Rule::Constraint(rule);
    app.menu.constrain.open = Some(Bar {
        at: at + vec2(OFFSET, OFFSET),
        picks: Picks::Pair { params: [Value::Null, Value::Null] },
        rules: Vec::new(),
        chosen: None,
        offset: "0 mm".into(),
        angle: "90 deg".into(),
        flip: false,
        focus: false,
        error: None,
        made: Some(Made { name, rule, value }),
        rect: None,
    });
    app.menu.constrain.seen = app.session.selection.clone();
}

/// The name of the constraint the open bar made (or was offered), when it has one.
pub(crate) fn made_name(app: &SolveApp) -> Option<String> {
    app.menu.constrain.open.as_ref().and_then(|b| b.made.as_ref()).map(|m| m.name.clone())
}

/// Close the bar and take the selection as seen, so it does not open again for it.
pub(crate) fn dismiss(app: &mut SolveApp) {
    app.menu.constrain.open = None;
    app.menu.constrain.seen = app.session.selection.clone();
}

/// Whether the bar is open.
pub fn is_open(app: &SolveApp) -> bool {
    app.menu.constrain.open.is_some()
}

/// One frame: opens the bar when the selection becomes picks a rule fits, closes it when the
/// selection changes or something else takes the pointer, and draws it.
pub fn show(app: &mut SolveApp, ctx: &egui::Context) {
    // The click that made the picks opens the bar: it is not a click elsewhere.
    let mut fresh = false;
    if app.session.selection != app.menu.constrain.seen {
        app.menu.constrain.seen = app.session.selection.clone();
        app.menu.constrain.open = None;
        // Only where the person is clicking: over the viewport.
        let at = ctx.input(|i| i.pointer.latest_pos()).filter(|_| app.viewport.mouse.is_some());
        if let Some(at) = at
            && free(app)
        {
            app.menu.constrain.open = bar_for(app, at);
            fresh = true;
        }
    }
    let Some(mut bar) = app.menu.constrain.open.take() else { return };
    if !free(app) || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        return;
    }
    let mut run: Option<Rule> = None;
    let mut change: Option<Change> = None;
    let pair = matches!(bar.picks, Picks::Pair { .. });
    let pointer = ctx.input(|i| i.pointer.latest_pos());
    // Once it has made a constraint it fades while the pointer is off it.
    let lit = bar.made.is_none() || bar.rect.zip(pointer).is_some_and(|(r, p)| r.expand(8.0).contains(p));
    let resp = Container::new(ctx, Surface::Popup)
        .area(egui::Id::unique("sc_quick_constrain"))
        .fixed_pos(bar.at)
        .constrain(true)
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            ui.multiply_opacity(if lit { 1.0 } else { FADED });
            if let Some(made) = bar.made.as_mut() {
                made_row(ui, made, &mut change);
                if let Some(e) = &bar.error {
                    crate::ui_kit::status_line(ui, e, crate::ui_kit::StatusTone::Error);
                }
                return;
            }
            ui.horizontal(|ui| {
                for (i, rule) in bar.rules.clone().into_iter().enumerate() {
                    // The likeliest is marked, until a rule with a value is chosen.
                    let mut on = bar.chosen.map_or(i == 0, |c| c == rule);
                    let r = crate::ui_kit::toggle_icon(ui, "quick_constrain", rule.label(), &mut on, rule.icon());
                    if r.clicked() {
                        if rule.takes_value() {
                            bar.chosen = Some(rule);
                            bar.focus = true;
                        } else {
                            run = Some(rule);
                        }
                    }
                }
                if pair {
                    ui.separator();
                    crate::ui_kit::toggle_icon(ui, "quick_constrain", solidcraft_ui_text::tr("constrain.flip"), &mut bar.flip, "reverse_normal");
                }
                if let Some(rule) = bar.chosen {
                    let text = if rule == Rule::Constraint(Constraint::Angle) { &mut bar.angle } else { &mut bar.offset };
                    let field = ui.add(egui::TextEdit::singleline(text).desired_width(72.0));
                    crate::scenario::publish_handle("quick_constrain:value", field.rect.center());
                    if std::mem::take(&mut bar.focus) {
                        field.request_focus();
                    }
                    let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    let tick = crate::ui_kit::button(ui, solidcraft_ui_text::tr("constrain.apply"), crate::ui_kit::Button::Normal);
                    crate::scenario::publish_handle("quick_constrain:apply", tick.rect.center());
                    if enter || tick.clicked() {
                        run = Some(rule);
                    }
                }
            });
            if let Some(e) = &bar.error {
                crate::ui_kit::status_line(ui, e, crate::ui_kit::StatusTone::Error);
            }
        });
    bar.rect = Some(resp.response.rect);
    // A press anywhere else closes it (the click goes on to select as it would); once it has
    // made a constraint, so does the pointer going well away from it.
    let outside = ctx.input(|i| i.pointer.any_pressed()) && !resp.response.contains_pointer();
    let away = bar.made.is_some() && crate::quick_menu::left(resp.response.rect, pointer);
    if !fresh && change.is_none() && (outside || away) {
        return;
    }
    if let Some(change) = change {
        match changed(app, &mut bar, change) {
            Ok(true) => {}
            // Deleted: nothing left to change.
            Ok(false) => return,
            Err(e) => bar.error = Some(e),
        }
    }
    if let Some(rule) = run {
        match apply(app, &bar, rule) {
            // Made: the picks are used up; the bar stays, faded, to flip or reverse what it made.
            Ok(name) => {
                let _ = app.run("select.clear", json!({}));
                app.menu.constrain.seen = app.session.selection.clone();
                let value = if rule == Rule::Constraint(Constraint::Angle) { bar.angle.clone() } else { bar.offset.clone() };
                bar.made = Some(Made { name, rule, value });
                bar.error = None;
            }
            Err(e) => bar.error = Some(e),
        }
    }
    app.menu.constrain.open = Some(bar);
}

/// A change asked of the constraint the bar made.
enum Change {
    Flip,
    Reverse,
    Value,
    Delete,
}

/// The row for the constraint the bar made: its name, Flip, its value and Reverse (Distance and
/// Angle), and Delete.
fn made_row(ui: &mut egui::Ui, made: &mut Made, change: &mut Option<Change>) {
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(18.0, 18.0), egui::Sense::hover());
        crate::ui_kit::icons::paint_icon(ui.painter(), r, made.rule.icon(), 1.0);
        ui.label(&made.name);
        ui.separator();
        let flip = crate::ui_kit::button(ui, solidcraft_ui_text::tr("constrain.flip"), crate::ui_kit::Button::Normal);
        crate::scenario::publish_handle("quick_constrain:made:flip", flip.rect.center());
        if flip.clicked() {
            *change = Some(Change::Flip);
        }
        if made.rule.takes_value() {
            let field = ui.add(egui::TextEdit::singleline(&mut made.value).desired_width(72.0));
            crate::scenario::publish_handle("quick_constrain:made:value", field.rect.center());
            if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                *change = Some(Change::Value);
            }
            let reverse = crate::ui_kit::button(ui, "Reverse", crate::ui_kit::Button::Normal);
            crate::scenario::publish_handle("quick_constrain:made:reverse", reverse.rect.center());
            if reverse.clicked() {
                *change = Some(Change::Reverse);
            }
        }
        let (r, _) = ui.allocate_exact_size(vec2(20.0, 20.0), egui::Sense::hover());
        let delete = crate::ui_kit::icon_button_at(ui, r, "delete", "Delete");
        crate::scenario::publish_handle("quick_constrain:made:delete", r.center());
        if delete.clicked() {
            *change = Some(Change::Delete);
        }
    });
}

/// `value` the other way: a plain number with its sign changed, anything else negated whole.
fn negated(value: &str) -> String {
    let v = value.trim();
    if let Some(inner) = v.strip_prefix("-(").and_then(|x| x.strip_suffix(')')) {
        return inner.to_string();
    }
    if let Some(rest) = v.strip_prefix('-')
        && !rest.contains(['+', '-', '*', '/'])
    {
        return rest.to_string();
    }
    if v.contains(['+', '-', '*', '/']) { format!("-({v})") } else { format!("-{v}") }
}

/// Make `change` to the constraint the bar made: whether it is still there.
fn changed(app: &mut SolveApp, bar: &mut Bar, change: Change) -> Result<bool, String> {
    let Some(made) = bar.made.as_mut() else { return Ok(false) };
    let key = if made.rule == Rule::Constraint(Constraint::Angle) { "angle" } else { "offset" };
    match change {
        Change::Flip => {
            app.run("assembly.constraint.edit", json!({"constraint": made.name, "flip": true}))?;
        }
        Change::Reverse => {
            let v = negated(&made.value);
            app.run("assembly.constraint.edit", json!({"constraint": made.name, key: v}))?;
            made.value = v;
        }
        Change::Value => {
            app.run("assembly.constraint.edit", json!({"constraint": made.name, key: made.value.trim()}))?;
        }
        Change::Delete => {
            app.run("joint.delete", json!({"joint": made.name}))?;
            return Ok(false);
        }
    }
    bar.error = None;
    Ok(true)
}

/// Add `rule` for the bar's picks, as one undo step named after it: the name of the constraint
/// that keeps it.
fn apply(app: &mut SolveApp, bar: &Bar, rule: Rule) -> Result<String, String> {
    match (&bar.picks, rule) {
        (Picks::Pair { params: [a, b] }, Rule::Constraint(c)) => {
            let mut p = json!({"type": c.name(), "a": a, "b": b});
            if bar.flip {
                p["flip"] = json!(true);
            }
            match c {
                Constraint::Angle => p["angle"] = json!(bar.angle.trim()),
                Constraint::Distance => p["offset"] = json!(bar.offset.trim()),
                _ => {}
            }
            app.run("assembly.constraint.create", p).and_then(|r| name_of(&r))
        }
        (Picks::Width { a, b }, Rule::Width) => one_step(app, rule.label(), |app| {
            let da = datum(app, a[0].part, "construct.plane.midplane", json!({"a": face(&a[0]), "b": face(&a[1])}))?;
            let db = datum(app, b[0].part, "construct.plane.midplane", json!({"a": face(&b[0]), "b": face(&b[1])}))?;
            let p = json!({"a": {"occurrence": a[0].part, "plane": da}, "b": {"occurrence": b[0].part, "plane": db}, "offset": "0 mm"});
            upright(app, b[0].part, p)
        }),
        (Picks::Symmetric { held, moved, plane }, Rule::Symmetric) => one_step(app, rule.label(), |app| {
            // The held face's mirror: as far beyond the plane as the face is before it, facing back.
            let n = held.snap.z;
            let along = (held.snap.at - plane.snap.at).dot(n);
            if !along.is_finite() {
                return Err("the plane and the face are too far apart".into());
            }
            let mirror = datum(app, held.part, "construct.plane.offset", json!({"base": face(held), "offset": format!("{} mm", -2.0 * along)}))?;
            let p = json!({"a": {"occurrence": held.part, "plane": mirror}, "b": moved.snap.param, "offset": "0 mm"});
            upright(app, moved.part, p)
        }),
        _ => Err("these picks don't fit that rule".into()),
    }
}

/// A face pick as construction commands take it: the point picked on it.
fn face(p: &Pick) -> Value {
    json!({"face": p.snap.param.get("face").cloned().unwrap_or_else(|| json!([p.snap.at.x, p.snap.at.y, p.snap.at.z]))})
}

/// Make a construction plane with `cmd` in the part `part` (activated for it, the active part put
/// back after): its name.
fn datum(app: &mut SolveApp, part: u64, cmd: &str, params: Value) -> Result<String, String> {
    let s = &app.session;
    let (active, path) = (s.active_component, s.active_occurrence_path.clone());
    let activate = if part == 0 {
        json!({"component": "root"})
    } else {
        let component = s.doc.occurrences.iter().find(|o| o.id == part).map(|o| o.component).ok_or("the part is gone")?;
        json!({"component": component, "occurrence": part})
    };
    app.run("component.activate", activate)?;
    let made = app.run(cmd, params);
    let back = match &path {
        Some(path) if active != 0 => json!({"component": active, "path": path}),
        _ => json!({"component": if active == 0 { json!("root") } else { json!(active) }}),
    };
    let _ = app.run("component.activate", back);
    let made = made?;
    made.get("name").and_then(Value::as_str).map(str::to_string).ok_or_else(|| "the plane was not made".into())
}

/// How far the placement of `part` is turned from `before`: the smallest cosine between an axis of
/// its frame then and now (1: not turned; −1: turned over).
fn turned(app: &SolveApp, part: u64, before: &solidcraft_engine::doc::Mat) -> f64 {
    let Some(now) = world(app, part) else { return 1.0 };
    (0..3).map(|i| (0..3).map(|k| before[i][k] * now[i][k]).sum::<f64>()).fold(1.0, f64::min)
}

/// Where part `part` is placed now (its first placement), pending moves included.
fn world(app: &SolveApp, part: u64) -> Option<solidcraft_engine::doc::Mat> {
    let s = &app.session;
    let parent = s.doc.occurrences.iter().find(|o| o.id == part)?.parent;
    let mut path = s.doc.first_path(parent);
    path.push(part);
    s.occurrence_placement(&path).ok().map(|p| p.world)
}

/// Coincide the two planes of `params` (`a`, `b`, `offset`) so the moving part `moving` is not
/// turned over: mated, or flush when a mate would turn it over (which way a datum plane faces is
/// the datum's own, not the face's), whichever turns it less.
fn upright(app: &mut SolveApp, moving: u64, params: Value) -> Result<String, String> {
    let before = world(app, moving);
    let mut tried = Vec::new();
    for rule in [Constraint::Mate, Constraint::Flush] {
        let mut p = params.clone();
        p["type"] = json!(rule.name());
        let made = app.run("assembly.constraint.create", p)?;
        let Some(before) = &before else { return name_of(&made) };
        let t = turned(app, moving, before);
        if t > 0.5 {
            return name_of(&made);
        }
        tried.push((t, rule));
        if let Some(id) = made.get("joint") {
            app.run("joint.delete", json!({"joint": id}))?;
        }
    }
    // Both turn it: the one that turns it less.
    let best = tried.into_iter().max_by(|a, b| a.0.total_cmp(&b.0)).map_or(Constraint::Mate, |(_, r)| r);
    let mut p = params;
    p["type"] = json!(best.name());
    app.run("assembly.constraint.create", p).and_then(|r| name_of(&r))
}

/// The name a create command gave what it made.
fn name_of(reply: &Value) -> Result<String, String> {
    reply.get("name").and_then(Value::as_str).map(str::to_string).ok_or_else(|| "the constraint was not made".into())
}

/// Run `body`'s commands as one undo step labelled `label`: all of them, or (on a refusal) none.
pub(crate) fn one_step<T>(app: &mut SolveApp, label: &str, body: impl FnOnce(&mut SolveApp) -> Result<T, String>) -> Result<T, String> {
    let base = app.session.undo.len();
    let checkpoint = app.session.execute("edit.checkpoint", &json!({})).ok().and_then(|r| r["checkpoint"].as_u64());
    let done = body(app);
    match (&done, checkpoint) {
        (Ok(_), Some(c)) => {
            let _ = app.session.execute("edit.restore_checkpoint", &json!({"checkpoint": c, "forget": true}));
        }
        (Err(_), Some(c)) => {
            let _ = app.session.execute("edit.restore_checkpoint", &json!({"checkpoint": c}));
        }
        _ => {}
    }
    // The first step holds the design from before them all: undo takes every one back.
    app.session.undo.truncate(base + usize::from(done.is_ok()));
    if done.is_ok()
        && let Some(step) = app.session.undo.get_mut(base)
    {
        step.label = label.to_string();
    }
    done
}

#[cfg(test)]
#[path = "quick_constrain_tests.rs"]
mod tests;
