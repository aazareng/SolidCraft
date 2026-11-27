//! Stored preferences on the Shortcuts page (`PAGE_SHORTCUTS`).
//!
//! Each entry is one field of `Prefs`, in `preference_settings!`'s form:
//! `field: Type = default, PAGE[, accept(range)][ => control(...)];`. Add a new preference
//! to the file of the page whose Defaults resets it, in alphabetical order by field name, so
//! two new preferences rarely land on the same line.

macro_rules! shortcuts_preferences {
    ($next:ident $(, $rest:ident)* ; $($fields:tt)*) => {
        $next! { $($rest),* ; $($fields)*
            /// Where the breadcrumbs were dragged to (screen points); none: below the ribbon, right of
            /// the Browser.
            breadcrumbs_at: Option<[f32; 2]> = None, PAGE_SHORTCUTS;
            /// Dragging a part joined to other free parts moves only that part, as far as its joints
            /// allow, the others held where they are (as in SolidWorks). Off: the whole chain moves as one.
            drag_moves_only_part: bool = true, PAGE_SHORTCUTS;
            /// One pick on a placed part in an assembly opens a card beside the pointer: its trail, the
            /// pattern or mirror that made it, and its joints to edit, suppress or delete. On by default.
            pick_card: bool = true, PAGE_SHORTCUTS;
            preview_reselect_modifier: crate::tool_modifiers::PreviewModifier = Default::default(), PAGE_SHORTCUTS;
            /// The shortcut bar's tools by mode, where the person changed them (S opens it).
            shortcut_bars: crate::shortcut_bar::Config = Default::default(), PAGE_SHORTCUTS;
            /// While a sketch is open, only sketch keys and keys that work everywhere run; a model
            /// command's key (E, F) does nothing. Off (the default): a model key pressed in a sketch
            /// runs its command, and one that builds a feature finishes the sketch first (#2815).
            sketch_keys_only: bool = false, PAGE_SHORTCUTS;
        }
    };
}
