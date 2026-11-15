/// IME event.
///
/// See <https://docs.rs/winit/latest/winit/event/enum.Ime.html>
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub enum ImeEvent {
    /// Notifies when the IME was enabled.
    #[deprecated = "No longer used by egui"]
    Enabled,

    /// A new IME candidate is being suggested.
    ///
    /// An empty preedit string indicates that the IME has been dismissed, while
    /// a non-empty preedit string indicates that the IME is active.
    Preedit {
        text: String,
        active_range_chars: Option<core::ops::Range<usize>>,
    },

    /// IME composition ended with this final result.
    ///
    /// The IME is considered dismissed after this event.
    Commit(String),

    /// Notifies when the text surrounding the cursor should be deleted.
    ///
    /// `before_chars` and `after_chars` are the number of characters (not
    /// bytes) to delete before and after the cursor, respectively.
    DeleteSurrounding {
        before_chars: usize,
        after_chars: usize,
    },

    /// The integration's native text field replaced the characters in `range_chars` with
    /// `text`, and the cursor is now after `text`.
    ///
    /// Only sent while the integration mirrors the field (see
    /// [`crate::RawInput::text_edit_mirror`]). `range_chars` counts characters (not bytes) of
    /// the whole text, as in [`crate::output::TextEditMirror::text`].
    ReplaceRange {
        range_chars: core::ops::Range<usize>,
        text: String,
    },

    /// The integration's native text field selected from `anchor_chars` to `focus_chars`.
    ///
    /// Only sent while the integration mirrors the field (see
    /// [`crate::RawInput::text_edit_mirror`]). The indices count characters (not bytes), and
    /// `focus_chars` is where the cursor is, which may come before `anchor_chars`.
    SetSelection {
        anchor_chars: usize,
        focus_chars: usize,
    },

    /// Notifies when the IME was disabled.
    #[deprecated = "No longer used by egui"]
    Disabled,
}
