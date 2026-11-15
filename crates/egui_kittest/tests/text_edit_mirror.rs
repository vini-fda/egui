//! A `TextEdit` mirrored in an integration's native text field (`RawInput::text_edit_mirror`).

use egui::{Event, ImeEvent, accesskit::Role, output::TextEditMirror};
use egui_kittest::Harness;
use kittest::Queryable as _;

fn singleline_harness(text: &str, mirror: bool) -> Harness<'static, String> {
    let mut harness = Harness::new_ui_state(
        |ui, text: &mut String| {
            ui.add(egui::TextEdit::singleline(text).hint_text("Address"));
        },
        text.to_owned(),
    );
    harness.input_mut().text_edit_mirror = mirror;
    harness.get_by_role(Role::TextInput).focus();
    harness.run();
    harness
}

fn mirror(harness: &Harness<'_, String>) -> Option<TextEditMirror> {
    harness.output().platform_output.text_edit_mirror.clone()
}

#[test]
fn describes_the_focused_field_only_when_asked() {
    let harness = singleline_harness("héllo", false);
    assert!(harness.output().platform_output.ime.is_some());
    assert_eq!(mirror(&harness), None);

    let harness = singleline_harness("héllo", true);
    let mirror = mirror(&harness).expect("the focused field should be mirrored");
    assert_eq!(mirror.text, "héllo");
    assert_eq!(mirror.scale, 1.0);
    let ime = harness.output().platform_output.ime.expect("IME output");
    assert!(
        ime.rect.contains(mirror.text_origin + egui::vec2(1.0, 1.0)),
        "the text starts inside the field: {:?} in {:?}",
        mirror.text_origin,
        ime.rect
    );
}

#[test]
fn multiline_fields_are_not_mirrored() {
    let mut harness = Harness::new_ui_state(
        |ui, text: &mut String| {
            ui.add(egui::TextEdit::multiline(text).hint_text("Notes"));
        },
        "two\nrows".to_owned(),
    );
    harness.input_mut().text_edit_mirror = true;
    harness.get_by_role(Role::MultilineTextInput).focus();
    harness.run();
    assert!(harness.output().platform_output.ime.is_some());
    assert_eq!(mirror(&harness), None);
}

#[test]
fn replace_range_counts_characters() {
    let mut harness = singleline_harness("héllo wörld", true);

    // "wörld" is characters 6..11, but bytes 7..13.
    harness.event(Event::Ime(ImeEvent::ReplaceRange {
        range_chars: 6..11,
        text: "🌍".to_owned(),
    }));
    harness.run();

    assert_eq!(harness.state(), "héllo 🌍");
    let mirror = mirror(&harness).expect("mirrored");
    assert_eq!(mirror.text, "héllo 🌍");
    assert_eq!((mirror.anchor_chars, mirror.focus_chars), (7, 7));
}

#[test]
fn replace_range_deletes_and_keeps_to_one_line() {
    let mut harness = singleline_harness("abcdef", true);

    harness.event(Event::Ime(ImeEvent::ReplaceRange {
        range_chars: 1..3,
        text: String::new(),
    }));
    harness.run();
    assert_eq!(harness.state(), "adef");

    harness.event(Event::Ime(ImeEvent::ReplaceRange {
        range_chars: 4..4,
        text: "x\ny".to_owned(),
    }));
    harness.run();
    assert_eq!(
        harness.state(),
        "adefx y",
        "a newline becomes a space, as in a paste"
    );

    // A range past the end is clamped to it.
    harness.event(Event::Ime(ImeEvent::ReplaceRange {
        range_chars: 5..99,
        text: "!".to_owned(),
    }));
    harness.run();
    assert_eq!(harness.state(), "adefx!");
}

#[test]
fn set_selection_keeps_its_direction() {
    let mut harness = singleline_harness("hello world", true);

    harness.event(Event::Ime(ImeEvent::SetSelection {
        anchor_chars: 11,
        focus_chars: 6,
    }));
    harness.run();

    let mirror = mirror(&harness).expect("mirrored");
    assert_eq!((mirror.anchor_chars, mirror.focus_chars), (11, 6));
    assert_eq!(harness.state(), "hello world", "selecting changes no text");

    // Typing replaces the selection, as it would in the native field.
    harness.event(Event::Text("there".to_owned()));
    harness.run();
    assert_eq!(harness.state(), "hello there");
}

#[test]
fn ime_events_need_the_focus() {
    let mut harness = Harness::new_ui_state(
        |ui, text: &mut String| {
            ui.add(egui::TextEdit::singleline(text).hint_text("Address"));
        },
        "abc".to_owned(),
    );
    harness.input_mut().text_edit_mirror = true;
    harness.run();

    harness.event(Event::Ime(ImeEvent::ReplaceRange {
        range_chars: 0..3,
        text: "xyz".to_owned(),
    }));
    harness.run();
    assert_eq!(harness.state(), "abc");
    assert_eq!(mirror(&harness), None);
}

#[test]
fn a_tap_puts_the_cursor_where_it_lands() {
    let mut harness = Harness::new_ui_state(
        |ui, text: &mut String| {
            ui.add(egui::TextEdit::singleline(text).hint_text("Greeting"));
        },
        "hello world".to_owned(),
    );
    harness.input_mut().text_edit_mirror = true;
    harness.run();
    let field = harness.get_by_role(Role::TextInput).rect();
    // On the text, around "hello": short of its end, where egui puts a cursor by default.
    let pos = egui::pos2(field.left() + 20.0, field.center().y);

    // A finger, as eframe on a touch screen sends it. Then an unfocused field senses only
    // clicks, so that dragging it scrolls.
    let touch = |phase| Event::Touch {
        device_id: egui::TouchDeviceId(1),
        id: egui::TouchId(1),
        phase,
        pos,
        force: None,
    };
    let button = |pressed| Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    };
    harness.event(Event::PointerMoved(pos));
    harness.event(touch(egui::TouchPhase::Start));
    harness.event(button(true));
    // One frame: running until settled would wait out the finger, into a long press.
    harness.step();
    harness.event(touch(egui::TouchPhase::End));
    harness.event(button(false));
    harness.run();

    let mirror = mirror(&harness).expect("the tapped field should be focused, and mirrored");
    assert_eq!(mirror.anchor_chars, mirror.focus_chars);
    assert!(
        (1..5).contains(&mirror.focus_chars),
        "the cursor should be where the tap was, in \"hello\", not at {}",
        mirror.focus_chars
    );
}

#[test]
fn the_text_origin_is_where_aligned_text_starts() {
    let mut harness = Harness::new_ui_state(
        |ui, text: &mut String| {
            ui.add(
                egui::TextEdit::singleline(text)
                    .hint_text("Value")
                    .desired_width(200.0)
                    .align(egui::Align2::CENTER_CENTER),
            );
        },
        "1.00".to_owned(),
    );
    harness.input_mut().text_edit_mirror = true;
    harness.get_by_role(Role::TextInput).focus();
    harness.run();

    let mirror = mirror(&harness).expect("mirrored");
    let ime = harness.output().platform_output.ime.expect("IME output");
    // A centered galley lies left of its own origin, by half its width. The text, which fits,
    // starts where its clip does.
    assert!(
        (mirror.text_origin.x - ime.rect.left()).abs() < 0.5,
        "centered text starts at the left of its clip: {:?} in {:?}",
        mirror.text_origin,
        ime.rect
    );
}
