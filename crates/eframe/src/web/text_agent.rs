//! The text agent is a hidden `<input>` element used to capture
//! IME and mobile keyboard input events.

use core::cell::RefCell;
use std::rc::Rc;

use wasm_bindgen::prelude::*;

use super::{AppRunner, WebRunner};

pub struct TextAgent {
    input: web_sys::HtmlInputElement,
    input_state: Rc<RefCell<InputState>>,
}

impl TextAgent {
    /// Attach the agent to the document.
    pub fn attach(
        runner_ref: &WebRunner,
        canvas: &web_sys::HtmlCanvasElement,
    ) -> Result<Self, JsValue> {
        let window = web_sys::window().unwrap();
        let document = window.document().unwrap();

        // create an `<input>` element
        let input = document
            .create_element("input")?
            .dyn_into::<web_sys::HtmlInputElement>()?;
        input.set_type("text");
        input.set_attribute("autocapitalize", "off")?;
        let input_state = Rc::new(RefCell::new(InputState::new(input.clone())));

        // Hide the element, and park it over the top-left corner of the canvas
        // so that focusing it can never scroll some other part
        // of the page into view.
        let style = input.style();
        style.set_property("background-color", "transparent")?;
        style.set_property("color", "transparent")?;
        style.set_property("opacity", "0")?;
        style.set_property("pointer-events", "none")?;
        style.set_property("border", "none")?;
        style.set_property("outline", "none")?;
        style.set_property("width", "1px")?;
        style.set_property("height", "1px")?;
        style.set_property("caret-color", "transparent")?;
        style.set_property("position", "absolute")?;
        // So that `width` includes the padding that lines the caret up with egui's
        // (see `InputState::update`).
        style.set_property("box-sizing", "border-box")?;
        style.set_property("padding", "0")?;
        style.set_property("top", &format!("{}px", canvas.offset_top()))?;
        style.set_property("left", &format!("{}px", canvas.offset_left()))?;
        // Prevent auto-zoom on mobile browsers (requires at least 16px).
        style.set_property("font-size", "16px")?;

        // Insert the input as a sibling of the canvas, so that its
        // `position: absolute` resolves against the same containing block
        // as the canvas' `offset_top`/`offset_left`.
        // This anchors the input to the canvas regardless of how the page
        // is scrolled or how the canvas is embedded, and also works when
        // the canvas is inside a shadow DOM.
        if let Some(parent) = canvas.parent_node() {
            parent.insert_before(&input, canvas.next_sibling().as_ref())?;
        } else if let Some(body) = document.body() {
            log::warn!("Canvas has no parent element - appending text agent to document body");
            body.append_child(&input)?;
        }

        // Focus the app on startup, without scrolling the page.
        // We do this instead of setting the `autofocus` attribute,
        // since the browser scrolls the focused element into view when
        // honoring `autofocus`, and there is no way to prevent that.
        // See https://github.com/emilk/egui/issues/8295
        super::focus_without_scroll(&input).ok();

        // attach event listeners

        runner_ref.add_event_listener(&input, "compositionstart", {
            let input_state = Rc::clone(&input_state);
            move |_: web_sys::CompositionEvent, runner: &mut AppRunner| {
                // The input is not written to while a composition is under way:
                input_state.borrow_mut().composing = true;
                // Repaint moves the text agent into place,
                // see `AppRunner::handle_platform_output`, which calls
                // `TextAgent::update`.
                runner.needs_repaint.repaint_asap();
            }
        })?;

        runner_ref.add_event_listener(&input, "input", {
            let input_state = Rc::clone(&input_state);
            move |event: web_sys::InputEvent, runner: &mut AppRunner| {
                input_state.borrow_mut().handle_input_event(&event, runner);
            }
        })?;
        runner_ref.add_event_listener(&input, "compositionend", {
            let input_state = Rc::clone(&input_state);
            move |_event: web_sys::CompositionEvent, runner: &mut AppRunner| {
                let mut input_state = input_state.borrow_mut();
                input_state.composing = false;
                if input_state.mirror.is_some() {
                    input_state.read_mirror(runner, false);
                } else {
                    input_state.handle_composition_end_event(runner);
                }
            }
        })?;

        // While the agent mirrors a `TextEdit` (see `InputState::update`), the browser moves
        // the caret and selection itself, on touches, keys and its text menu:
        runner_ref.add_event_listener(&document, "selectionchange", {
            let input_state = Rc::clone(&input_state);
            move |_event: web_sys::Event, runner: &mut AppRunner| {
                let mut input_state = input_state.borrow_mut();
                if input_state.mirror.is_some()
                    && super::has_focus(&input_state.input)
                    && !input_state.composing
                {
                    input_state.read_mirror(runner, false);
                }
            }
        })?;

        // Writing the mirror waits while a finger is on it (see `InputState::write_mirror`):
        // it would move the handle being dragged, or close the text menu being opened.
        runner_ref.add_event_listener(&input, "touchstart", {
            let input_state = Rc::clone(&input_state);
            move |_event: web_sys::TouchEvent, _runner: &mut AppRunner| {
                input_state.borrow_mut().finger_down = true;
            }
        })?;
        for event_name in ["touchend", "touchcancel"] {
            runner_ref.add_event_listener(&input, event_name, {
                let input_state = Rc::clone(&input_state);
                move |event: web_sys::TouchEvent, runner: &mut AppRunner| {
                    if event.touches().length() == 0 {
                        input_state.borrow_mut().finger_down = false;
                        // Make the write that waited:
                        runner.needs_repaint.repaint_asap();
                    }
                }
            })?;
        }

        // While the agent lies over the focused `TextEdit` on a touch screen (see
        // `InputState::update`), the touches there land on the agent, not the canvas.
        // egui must still get them, to place the cursor and select:
        super::events::install_text_agent_touches(runner_ref, &input)?;

        runner_ref.add_event_listener(&input, "keydown", {
            let input_state = Rc::clone(&input_state);
            move |event: web_sys::KeyboardEvent, runner: &mut AppRunner| {
                let is_consumed = InputState::handle_keydown_event(&input_state, &event);
                if !is_consumed {
                    // The canvas doesn't get keydown/keyup events when the text agent is focused,
                    // so we need to forward them to the runner:
                    super::events::on_keydown(event, runner);
                }
            }
        })?;
        runner_ref.add_event_listener(&input, "keyup", {
            let input_state = Rc::clone(&input_state);
            move |event: web_sys::KeyboardEvent, runner: &mut AppRunner| {
                let is_consumed = InputState::handle_keyup_event(&input_state, &event);
                if !is_consumed {
                    // The canvas doesn't get keydown/keyup events when the text agent is focused,
                    // so we need to forward them to the runner:
                    super::events::on_keyup(event, runner);
                }
            }
        })?;

        Ok(Self { input, input_state })
    }

    pub fn update(
        &self,
        ime: Option<egui::output::IMEOutput>,
        mirror: Option<&egui::output::TextEditMirror>,
        canvas: &web_sys::HtmlCanvasElement,
        egui_ctx: &egui::Context,
    ) -> Result<(), JsValue> {
        self.input_state
            .borrow_mut()
            .update(ime, mirror, canvas, egui_ctx)
    }

    /// See [`Self::set_touch`].
    pub fn touch(&self) -> bool {
        self.input_state.borrow().touch
    }

    /// Whether the agent is a copy of the focused `TextEdit`, which the browser edits (see
    /// `egui::RawInput::text_edit_mirror`). Its touches, keys and clipboard are then the
    /// browser's, and the edits come back to egui from the agent.
    pub fn is_mirroring(&self) -> bool {
        self.input_state.borrow().mirror.is_some()
    }

    /// Whether the last pointer to go down was a finger or a pen, rather than a mouse.
    ///
    /// On a touch screen, the agent lies over the text of the focused `TextEdit`, so that
    /// touching the text touches a real `<input>`: holding it, or tapping its caret, opens the
    /// browser's own text menu, with Paste. Nothing else pastes on a phone, which has no
    /// keyboard shortcut for it (<https://github.com/emilk/egui/issues/501>).
    pub fn set_touch(&self, touch: bool) {
        self.input_state.borrow_mut().touch = touch;
    }

    /// Whether `target` (of a DOM event) is the agent.
    pub fn is_target(&self, target: Option<&web_sys::EventTarget>) -> bool {
        target.is_some_and(|target| self.input.unchecked_ref::<web_sys::EventTarget>() == target)
    }

    pub fn set_focus(&self, on: bool) {
        if on {
            self.focus();
        } else {
            self.blur();
        }
    }

    pub fn has_focus(&self) -> bool {
        super::has_focus(&self.input)
    }

    pub fn focus(&self) {
        if self.has_focus() {
            return;
        }

        log::trace!("Focusing text agent");

        // A copy of the `TextEdit` may lie where the on-screen keyboard will be. Letting the
        // focus scroll lets iOS pan the page to keep it in view, as for any text field:
        // touches behind the keyboard's area don't reach the page.
        let focused = if self.is_mirroring() {
            self.input.focus()
        } else {
            super::focus_without_scroll(&self.input)
        };
        if let Err(err) = focused {
            log::error!("failed to set focus: {}", super::string_from_js_value(&err));
        }
    }

    pub fn blur(&self) {
        if !self.has_focus() {
            return;
        }

        log::trace!("Blurring text agent");

        if let Err(err) = self.input.blur() {
            log::error!("failed to set focus: {}", super::string_from_js_value(&err));
        }
        self.input_state.borrow_mut().clear();
    }

    pub(crate) fn interrupt_ime_composition(&self) {
        let mut state = self.input_state.borrow_mut();
        if state.mirror.is_some() {
            // The agent holds the `TextEdit`'s text, not just a composition: writing that text
            // again ends the composition.
            if state.composing {
                state.composing = false;
                state.mirror_value.clear();
                state.write_mirror();
            }
        } else {
            state.clear();
        }
    }
}

impl Drop for TextAgent {
    fn drop(&mut self) {
        self.input.remove();
    }
}

struct InputState {
    input: web_sys::HtmlInputElement,
    last_text: String,
    ime_output: Option<egui::output::IMEOutput>,
    keydown_special_case: KeydownSpecialCase,

    /// See [`TextAgent::set_touch`].
    touch: bool,

    /// Whether the agent lies over the focused `TextEdit`, to take its touches.
    covers_text_edit: bool,

    /// The focused `TextEdit`, while the agent mirrors it (see [`TextAgent::is_mirroring`]).
    mirror: Option<egui::output::TextEditMirror>,

    /// The value eframe last read from the mirror, or wrote to it.
    mirror_value: String,

    /// The selection (anchor, focus), in characters, eframe last read from the mirror, or
    /// wrote to it.
    mirror_selection: (usize, usize),

    /// The mirror's `scrollLeft` that lines its text up with egui's.
    mirror_scroll_left: f64,

    /// Whether the browser is composing text in the agent (IME).
    composing: bool,

    /// Whether a finger is on the agent.
    finger_down: bool,

    /// The fonts of egui added to the page, as `FontFace`s for the mirror, by name.
    font_faces: std::collections::HashSet<String>,
}

#[derive(Clone, Copy)]
enum KeydownSpecialCase {
    None,

    /// On Android Gboard 14.7.09, when suggestions remain visible while typing
    /// letters without IME composition (e.g., Latin or Cyrillic), pressing
    /// Backspace produces key code 229 instead of the expected Backspace key
    /// code.
    /// Without the workaround, users have to press Backspace twice before text
    /// starts being deleted.
    ///
    /// This workaround is also required for Android Gboard corrections and
    /// completions (e.g., `tex|` -> `Texas`) to work correctly. In these
    /// cases, a `deleteContentBackward` input event fires first (e.g., to
    /// delete `tex`), followed by an `insertText` input event (e.g., to insert
    /// `Texas`).
    ///
    /// Since it is difficult to distinguish between a Backspace press and a
    /// correction or completion (e.g., when the state is `t|`, it is unclear
    /// whether the user wants to delete `t` or replace it with `Texas`), we
    /// send a `DeleteSurrounding` IME event in all cases instead of
    /// synthetically generating Backspace press and release events.
    AndroidKeycode229,

    /// iOS (18.6)'s built-in Korean keyboard uses `deleteContentBackward` to
    /// compose Hangul characters. In these cases, the key code is 0.
    IosKeycode0,
}

impl InputState {
    fn new(input: web_sys::HtmlInputElement) -> Self {
        Self {
            input,
            last_text: String::new(),
            ime_output: None,
            keydown_special_case: KeydownSpecialCase::None,
            touch: false,
            covers_text_edit: false,
            mirror: None,
            mirror_value: String::new(),
            mirror_selection: (0, 0),
            mirror_scroll_left: 0.0,
            composing: false,
            finger_down: false,
            font_faces: Default::default(),
        }
    }

    fn update(
        &mut self,
        ime: Option<egui::output::IMEOutput>,
        mirror: Option<&egui::output::TextEditMirror>,
        canvas: &web_sys::HtmlCanvasElement,
        egui_ctx: &egui::Context,
    ) -> Result<(), JsValue> {
        let covers_text_edit = self.touch && ime.is_some();
        let mirror = mirror.filter(|_| covers_text_edit);

        if self.mirror.is_some() != mirror.is_some() {
            // Starting or stopping to mirror: the value and selection are written afresh.
            self.clear();
        }

        // Don't move the text agent unless the position actually changed:
        if self.ime_output != ime
            || self.covers_text_edit != covers_text_edit
            || self.mirror.as_ref() != mirror
        {
            self.place(ime, mirror, covers_text_edit, canvas, egui_ctx)?;
        }

        self.write_mirror();
        Ok(())
    }

    fn place(
        &mut self,
        ime: Option<egui::output::IMEOutput>,
        mirror: Option<&egui::output::TextEditMirror>,
        covers_text_edit: bool,
        canvas: &web_sys::HtmlCanvasElement,
        egui_ctx: &egui::Context,
    ) -> Result<(), JsValue> {
        let zoom_factor = egui_ctx.zoom_factor();
        self.ime_output = ime;
        let style = self.input.style();
        if self.covers_text_edit != covers_text_edit {
            self.covers_text_edit = covers_text_edit;
            if !covers_text_edit {
                style.set_property("opacity", "0")?;
                style.set_property("pointer-events", "none")?;
                style.set_property("width", "1px")?;
                style.set_property("height", "1px")?;
                style.set_property("padding-left", "0")?;
            }
        }
        if self.mirror.is_some() && mirror.is_none() {
            self.unstyle_mirror()?;
        }
        self.mirror = mirror.cloned();

        let Some(ime) = ime else { return Ok(()) };

        // NOTE: we don't set the input's `type` to `password` based on
        // `ime.purpose`, because that would confuse some password managers.
        // For example, Chrome's password manager will always think the last
        // letter typed in the password field is the password.

        let native_ppp = super::native_pixels_per_point();

        // The input is a sibling of the canvas (see `attach`), so we position
        // it relative to the same containing block using the canvas offset.
        // Unlike `get_bounding_client_rect`, the offset is unaffected by page
        // scrolling, and doesn't flap when the virtual keyboard is shown on
        // mobile Safari.

        // Clamp the input position within the canvas width to prevent unwanted horizontal scrolling.
        let logical_canvas_width = canvas.width() as f32 / native_ppp;
        let visible_x = ime.cursor_rect.center().x * zoom_factor;
        let clamped_x = visible_x.clamp(0.0, logical_canvas_width);

        // Clamp the input position within the canvas height to prevent unwanted vertical scrolling.
        let logical_canvas_height = canvas.height() as f32 / native_ppp;
        let visible_y = ime.cursor_rect.center().y * zoom_factor;
        let clamped_y = visible_y.clamp(0.0, logical_canvas_height);

        if let Some(mirror) = mirror {
            let canvas_rect = egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(logical_canvas_width, logical_canvas_height),
            );
            self.style_mirror(&ime, mirror, canvas_rect, canvas, egui_ctx)?;
        } else if covers_text_edit {
            // Lie over the text edit's text (within the canvas), with the left padding putting
            // the input's own caret where egui's is: that is where the IME input points to,
            // and where the browser's text menu opens.
            let canvas_rect = egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(logical_canvas_width, logical_canvas_height),
            );
            let rect = (ime.rect * zoom_factor).intersect(canvas_rect);
            let padding_left = (clamped_x - rect.left()).clamp(0.0, rect.width());
            style.set_property(
                "left",
                &format!("{}px", canvas.offset_left() as f32 + rect.left()),
            )?;
            style.set_property(
                "top",
                &format!("{}px", canvas.offset_top() as f32 + rect.top()),
            )?;
            style.set_property("width", &format!("{}px", rect.width()))?;
            style.set_property("height", &format!("{}px", rect.height()))?;
            style.set_property("padding-left", &format!("{padding_left}px"))?;
            style.set_property("pointer-events", "auto")?;
            // Still invisible, its text, caret and background being transparent: iOS Safari
            // opens no text menu on an editable element it takes for hidden, at opacity 0.
            style.set_property("opacity", "1")?;
        } else {
            // This is where the IME input will point to:
            style.set_property(
                "left",
                &format!("{}px", canvas.offset_left() as f32 + clamped_x),
            )?;
            style.set_property(
                "top",
                &format!("{}px", canvas.offset_top() as f32 + clamped_y),
            )?;
        }

        Ok(())
    }

    fn clear(&mut self) {
        self.input.set_value("");
        self.last_text.clear();
        self.mirror_value.clear();
        self.mirror_selection = (0, 0);
    }

    /// Make the agent a copy of the focused `TextEdit`'s text, on top of it: same font,
    /// size and place, so that the browser's caret, selection, handles and magnifier land
    /// on egui's text. The agent's own text is transparent: egui draws the text.
    fn style_mirror(
        &mut self,
        ime: &egui::output::IMEOutput,
        mirror: &egui::output::TextEditMirror,
        canvas_rect: egui::Rect,
        canvas: &web_sys::HtmlCanvasElement,
        egui_ctx: &egui::Context,
    ) -> Result<(), JsValue> {
        let zoom_factor = egui_ctx.zoom_factor();
        let style = self.input.style();
        let (font_family, font_scale) = self.font_family(egui_ctx, &mirror.font_id.family);

        // Below 16px, iOS Safari zooms the page into a focused input. So the input is laid
        // out at 16px and scaled down instead, which it accepts.
        let font_size = mirror.font_id.size * font_scale * mirror.scale * zoom_factor;
        let (css_font_size, s) = if font_size < 16.0 {
            (16.0, font_size / 16.0)
        } else {
            (font_size, 1.0)
        };

        // The input covers the whole field, so that a touch anywhere on it is the browser's,
        // as on a native field. Its padding puts its text where egui's is, and clips it about
        // where egui clips it (to `ime.rect`).
        let rect = (mirror.field_rect * zoom_factor).intersect(canvas_rect);
        let text_rect = (ime.rect * zoom_factor).intersect(rect);
        let origin = mirror.text_origin * zoom_factor;
        let row_height = mirror.row_height * mirror.scale * zoom_factor;
        let padding_left = (origin.x.max(text_rect.left()) - rect.left()).max(0.0);
        // The browser keeps room for its caret after the text, and its text may come out a
        // little wider than egui's: when the text fills egui's clip, the agent would scroll by
        // itself. So give it some of the frame's room on the right. Its text is transparent.
        let padding_right = (rect.right() - text_rect.right() - row_height).max(0.0);
        let padding_top = (origin.y - rect.top()).clamp(0.0, rect.height());
        let height = rect.height().max(padding_top + row_height);
        let padding_bottom = (height - padding_top - row_height).max(0.0);
        // egui scrolls the text to the left when it doesn't fit:
        self.mirror_scroll_left = f64::from((text_rect.left() - origin.x).max(0.0) / s);

        let px = |x: f32| format!("{}px", x / s);
        style.set_property(
            "left",
            &format!("{}px", canvas.offset_left() as f32 + rect.left()),
        )?;
        style.set_property(
            "top",
            &format!("{}px", canvas.offset_top() as f32 + rect.top()),
        )?;
        style.set_property("width", &px(rect.width()))?;
        style.set_property("height", &px(height))?;
        style.set_property("padding-left", &px(padding_left))?;
        style.set_property("padding-right", &px(padding_right))?;
        style.set_property("padding-top", &px(padding_top))?;
        style.set_property("padding-bottom", &px(padding_bottom))?;
        style.set_property("line-height", &px(row_height))?;
        style.set_property("font-size", &format!("{css_font_size}px"))?;
        style.set_property("font-family", &font_family)?;
        style.set_property("font-variant-ligatures", "none")?;
        style.set_property("font-kerning", "normal")?;
        style.set_property("letter-spacing", "0")?;
        style.set_property("transform-origin", "0 0")?;
        if s == 1.0 {
            style.set_property("transform", "none")?;
        } else {
            style.set_property("transform", &format!("scale({s})"))?;
        }
        let [r, g, b, a] = mirror.cursor_color.to_srgba_unmultiplied();
        style.set_property(
            "caret-color",
            &format!("rgba({r}, {g}, {b}, {})", f32::from(a) / 255.0),
        )?;
        style.set_property("pointer-events", "auto")?;
        // Still invisible, its text and background being transparent: iOS Safari opens no
        // text menu on an editable element it takes for hidden, at opacity 0.
        style.set_property("opacity", "1")?;

        // A password field gets the browser's password handling: no Copy, and a double tap
        // selects a word's worth of dots rather than the whole value.
        if ime.purpose == egui::IMEPurpose::Password {
            if self.input.type_() != "password" {
                self.input.set_type("password");
                self.input
                    .set_attribute("autocomplete", "current-password")?;
            }
        } else if self.input.type_() != "text" {
            self.input.set_type("text");
            self.input.remove_attribute("autocomplete")?;
        }

        Ok(())
    }

    /// Undo [`Self::style_mirror`].
    fn unstyle_mirror(&self) -> Result<(), JsValue> {
        let style = self.input.style();
        for property in [
            "padding-right",
            "padding-top",
            "padding-bottom",
            "line-height",
            "font-family",
            "font-variant-ligatures",
            "font-kerning",
            "letter-spacing",
            "transform",
            "transform-origin",
        ] {
            style.remove_property(property)?;
        }
        style.set_property("padding-left", "0")?;
        style.set_property("font-size", "16px")?;
        style.set_property("caret-color", "transparent")?;
        if self.input.type_() != "text" {
            self.input.set_type("text");
            self.input.remove_attribute("autocomplete")?;
        }
        Ok(())
    }

    /// The CSS `font-family` of egui's `family`, with its whole fallback chain, and the scale
    /// egui gives its first font. Adds egui's fonts to the page, as needed.
    fn font_family(
        &mut self,
        egui_ctx: &egui::Context,
        family: &egui::FontFamily,
    ) -> (String, f32) {
        let document = web_sys::window().and_then(|w| w.document());
        egui_ctx.fonts(|fonts| {
            let definitions = fonts.definitions();
            let names = definitions
                .families
                .get(family)
                .map(Vec::as_slice)
                .unwrap_or_default();
            let mut css = Vec::with_capacity(names.len());
            for name in names {
                let css_name = format!("\"egui {}\"", name.replace(['"', '\\'], ""));
                if !self.font_faces.contains(name)
                    && let Some(data) = definitions.font_data.get(name)
                    && let Some(document) = &document
                {
                    let bytes: &[u8] = (*data.font).as_ref();
                    match web_sys::FontFace::new_with_u8_array(&css_name, bytes) {
                        Ok(face) => {
                            if let Err(err) = document.fonts().add(&face) {
                                log::warn!(
                                    "failed to add font {name:?}: {}",
                                    super::string_from_js_value(&err)
                                );
                            }
                        }
                        Err(err) => log::warn!(
                            "failed to load font {name:?}: {}",
                            super::string_from_js_value(&err)
                        ),
                    }
                    self.font_faces.insert(name.clone());
                }
                css.push(css_name);
            }
            css.push("monospace".to_owned());
            let scale = names
                .first()
                .and_then(|name| definitions.font_data.get(name))
                .map_or(1.0, |data| data.tweak.scale);
            (css.join(", "), scale)
        })
    }

    /// Write egui's text and selection to the mirror, if they changed since eframe last read
    /// or wrote them. Not while the browser is editing: during a composition, or with a
    /// finger on the agent (a write moves the handle being dragged); it waits until then.
    fn write_mirror(&mut self) {
        let Some(mirror) = &self.mirror else { return };
        if self.composing || self.finger_down {
            return;
        }

        let text_changed = mirror.text != self.mirror_value;
        if text_changed {
            self.input.set_value(&mirror.text);
            self.mirror_value.clone_from(&mirror.text);
        }

        let selection = (mirror.anchor_chars, mirror.focus_chars);
        if text_changed || selection != self.mirror_selection {
            let (anchor, focus) = selection;
            let start = utf16_offset(&mirror.text, anchor.min(focus));
            let end = utf16_offset(&mirror.text, anchor.max(focus));
            let direction = if focus < anchor {
                "backward"
            } else {
                "forward"
            };
            if let Err(err) = self
                .input
                .set_selection_range_with_direction(start, end, direction)
            {
                log::warn!(
                    "failed to select in the text agent: {}",
                    super::string_from_js_value(&err)
                );
            }
            self.mirror_selection = selection;
        }

        // Through `Reflect`: web-sys types `scrollLeft` differently with `web_sys_unstable_apis`.
        let scroll_left = js_sys::Reflect::get(&self.input, &"scrollLeft".into())
            .ok()
            .and_then(|value| value.as_f64())
            .unwrap_or_default();
        if (scroll_left - self.mirror_scroll_left).abs() >= 1.0 {
            js_sys::Reflect::set(
                &self.input,
                &"scrollLeft".into(),
                &self.mirror_scroll_left.round().into(),
            )
            .ok();
        }
    }

    /// Tell egui how the browser changed the mirror's text and selection.
    ///
    /// `force_selection`: send the selection even if it seems unchanged.
    fn read_mirror(&mut self, runner: &mut AppRunner, force_selection: bool) {
        let value = self.input.value();
        let text_changed = value != self.mirror_value;
        if text_changed {
            let (range_chars, text) = replaced_range(&self.mirror_value, &value);
            runner
                .input
                .raw
                .events
                .push(egui::Event::Ime(egui::ImeEvent::ReplaceRange {
                    range_chars,
                    text,
                }));
            self.mirror_value.clone_from(&value);
        }

        if let Some(selection) = self.read_selection(&value)
            && (text_changed || force_selection || selection != self.mirror_selection)
        {
            let (anchor_chars, focus_chars) = selection;
            runner
                .input
                .raw
                .events
                .push(egui::Event::Ime(egui::ImeEvent::SetSelection {
                    anchor_chars,
                    focus_chars,
                }));
            self.mirror_selection = selection;
        }

        runner.needs_repaint.repaint_asap();
    }

    /// The agent's selection (anchor, focus), in characters of `value`.
    fn read_selection(&self, value: &str) -> Option<(usize, usize)> {
        let start = self.input.selection_start().ok()??;
        let end = self.input.selection_end().ok()??;
        let start = char_offset(value, start);
        let end = char_offset(value, end);
        let backward =
            self.input.selection_direction().ok().flatten().as_deref() == Some("backward");
        Some(if backward { (end, start) } else { (start, end) })
    }

    fn handle_input_event(&mut self, event: &web_sys::InputEvent, runner: &mut AppRunner) {
        if self.mirror.is_some() {
            // Whatever the browser did (typing, a correction, a paste, a cut, a composition,
            // an edit it made for a key), the change is in the value:
            self.read_mirror(runner, false);
            return;
        }

        if self
            .ime_output
            .as_ref()
            .is_some_and(|ime| ime.purpose == egui::IMEPurpose::Password)
        {
            self.handle_input_event_password(event, runner);
            return;
        }

        let input_type = event.input_type();

        if !event.is_composing()
            && input_type != "insertText"
            // iOS uses this for corrections and completions (e.g., `tex|` ->
            // `Texas`).
            && input_type != "insertReplacementText"
            && (matches!(self.keydown_special_case, KeydownSpecialCase::None)
                || input_type != "deleteContentBackward")
        {
            self.clear();

            return;
        }

        let text = self.input.value();

        let prefix_len = longest_common_prefix_length(&text, &self.last_text);
        let last_text_len = self.last_text.chars().count();
        if prefix_len < last_text_len {
            let out_event = egui::Event::Ime(egui::ImeEvent::DeleteSurrounding {
                before_chars: last_text_len - prefix_len,
                after_chars: 0,
            });
            runner.input.raw.events.push(out_event);
        }

        let preedit_text: String = text.chars().skip(prefix_len).collect();
        let out_event = if event.is_composing() {
            // We handle the composition update here instead of in a
            // `compositionupdate` event because the selection range
            // has not yet been updated when `compositionupdate` fires.
            let active_range_chars = self.active_range_chars(&text, prefix_len);
            egui::Event::Ime(egui::ImeEvent::Preedit {
                text: preedit_text,
                active_range_chars,
            })
        } else {
            egui::Event::Text(preedit_text)
        };
        runner.input.raw.events.push(out_event);

        if event.is_composing() {
            self.last_text = text.chars().take(prefix_len).collect();
        } else {
            self.last_text = text;
        }

        runner.needs_repaint.repaint_asap();
    }

    fn handle_input_event_password(&mut self, event: &web_sys::InputEvent, runner: &mut AppRunner) {
        let input_type = event.input_type();

        if input_type != "insertText" {
            return;
        }

        let text = self.input.value();

        runner.input.raw.events.push(egui::Event::Text(text));
        self.clear();
    }

    /// Compute the active range (cursor or conversion segment) within the
    /// preedit text, based on the selection in the input element.
    ///
    /// `text` is the full `input.value()`, and `prefix_len_chars` is the
    /// number of chars at the start of `text` that are committed (not part
    /// of the preedit). `selectionStart`/`selectionEnd` are UTF-16 offsets
    /// within the full `input.value()`, so they are adjusted to be relative
    /// to the preedit text.
    fn active_range_chars(
        &self,
        text: &str,
        prefix_len_chars: usize,
    ) -> Option<core::ops::Range<usize>> {
        let selection_start = self.input.selection_start().unwrap_or(None)? as usize;
        let selection_end = self.input.selection_end().unwrap_or(None)? as usize;

        let text_utf16 = text.encode_utf16().collect::<Vec<u16>>();
        if selection_start > text_utf16.len() || selection_end > text_utf16.len() {
            // This can occur on Android Chrome. see discussion in:
            // <https://github.com/emilk/egui/pull/8045>.
            return None;
        }

        let text_before_selection = String::from_utf16_lossy(&text_utf16[..selection_start]);
        let text_in_selection =
            String::from_utf16_lossy(&text_utf16[selection_start..selection_end]);
        let count_before_selection = text_before_selection.chars().count();
        let count_in_selection = text_in_selection.chars().count();

        // Adjust for the committed prefix to get the range within the preedit text.
        let start = count_before_selection.saturating_sub(prefix_len_chars);
        let end = start + count_in_selection;
        Some(start..end)
    }

    fn handle_composition_end_event(&mut self, runner: &mut AppRunner) {
        let text = self.input.value();

        let commit_text = {
            let prefix_len = self.last_text.chars().count();
            text.chars().skip(prefix_len).collect::<String>()
        };
        let out_event = egui::Event::Ime(egui::ImeEvent::Commit(commit_text));
        runner.input.raw.events.push(out_event);

        self.last_text = text;

        runner.needs_repaint.repaint_asap();
    }

    /// ## Returns
    /// Whether the event is consumed. If `true`, the caller should not do
    /// further processing for this event.
    fn handle_keydown_event(input_state: &RefCell<Self>, event: &web_sys::KeyboardEvent) -> bool {
        // Platform-sniffing methods are unreliable, so they are not used as
        // guards here.
        let special_case = match event.key_code() {
            229 => KeydownSpecialCase::AndroidKeycode229,
            0 => KeydownSpecialCase::IosKeycode0,
            _ => KeydownSpecialCase::None,
        };
        input_state.borrow_mut().keydown_special_case = special_case;

        // https://web.archive.org/web/20200526195704/https://www.fxsitecompat.dev/en-CA/docs/2018/keydown-and-keyup-events-are-now-fired-during-ime-composition/
        if event.is_composing() || !matches!(special_case, KeydownSpecialCase::None) {
            true
        } else if input_state.borrow().mirror.is_some() {
            // The browser applies an editing key to the mirror, and the change comes back as
            // an `input` or `selectionchange` event; egui applying it too would do it twice.
            // Other keys (Enter, Tab, Escape, shortcuts) are egui's.
            is_editing_key(event)
        } else {
            if event.key().chars().count() > 1
                || event.ctrl_key()
                || event.alt_key()
                || event.meta_key()
            {
                input_state.borrow_mut().clear();
            }
            false
        }
    }

    /// ## Returns
    /// Whether the event is consumed. If `true`, the caller should not do
    /// further processing for this event.
    fn handle_keyup_event(input_state: &RefCell<Self>, event: &web_sys::KeyboardEvent) -> bool {
        input_state.borrow_mut().keydown_special_case = KeydownSpecialCase::None;

        // https://web.archive.org/web/20200526195704/https://www.fxsitecompat.dev/en-CA/docs/2018/keydown-and-keyup-events-are-now-fired-during-ime-composition/
        event.is_composing() || event.key_code() == 229
    }
}

/// Whether the browser edits the text, or moves in it, for this key.
fn is_editing_key(event: &web_sys::KeyboardEvent) -> bool {
    let key = event.key();
    if matches!(
        key.as_str(),
        "Backspace"
            | "Delete"
            | "ArrowLeft"
            | "ArrowRight"
            | "ArrowUp"
            | "ArrowDown"
            | "Home"
            | "End"
            | "PageUp"
            | "PageDown"
    ) {
        return true;
    }
    // Select all, copy, cut, paste, undo, redo, and the Emacs keys of macOS and iPadOS:
    let mut chars = key.chars();
    (event.ctrl_key() || event.meta_key())
        && matches!(
            (chars.next(), chars.next()),
            (Some(c), None) if "abcefhknpuvwxyz".contains(c.to_ascii_lowercase())
        )
}

/// The UTF-16 offset of the character at `char_offset` in `text`.
fn utf16_offset(text: &str, char_offset: usize) -> u32 {
    text.chars()
        .take(char_offset)
        .map(|c| c.len_utf16() as u32)
        .sum()
}

/// The character at the UTF-16 offset `utf16_offset` in `text`, or the one it falls inside.
fn char_offset(text: &str, utf16_offset: u32) -> usize {
    let mut utf16 = 0;
    text.chars()
        .take_while(|c| {
            utf16 += c.len_utf16() as u32;
            utf16 <= utf16_offset
        })
        .count()
}

/// How `old` became `new`: the characters of `old` replaced, and what by.
fn replaced_range(old: &str, new: &str) -> (core::ops::Range<usize>, String) {
    let old: Vec<char> = old.chars().collect();
    let new: Vec<char> = new.chars().collect();
    let prefix = core::iter::zip(&old, &new)
        .take_while(|(a, b)| a == b)
        .count();
    let suffix = core::iter::zip(old[prefix..].iter().rev(), new[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    (
        prefix..old.len() - suffix,
        new[prefix..new.len() - suffix].iter().collect(),
    )
}

fn longest_common_prefix_length(a: &str, b: &str) -> usize {
    core::iter::zip(a.chars(), b.chars())
        .take_while(|(a, b)| a == b)
        .count()
}
