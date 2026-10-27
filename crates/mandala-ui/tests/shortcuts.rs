//! Rice keyboard shortcuts: Ctrl+Space jumps to the command bar (the
//! rofi/SUPER+D launcher role) and Ctrl+T opens the Terminal, from both the
//! parked default focus and the focused command input. Modifier chords must
//! bubble past buttons without "clicking" them.
//!
//! Runs the real `MainWindow` on Slint's headless testing backend, so no
//! display or window system is involved. One test fn per file: the backend
//! can only be initialised once per process.

use mandala_ui::{populate_slots, MainWindow};
use slint::ComponentHandle;
use std::cell::RefCell;
use std::rc::Rc;

/// Dispatch one key press or release. The testing backend sends
/// `WindowEvent::KeyPressed/KeyReleased { text }`, which Slint's window
/// tracks into `KeyEvent.modifiers` (Control is U+0011 = key_codes::Control).
fn key_event(ui: &MainWindow, text: &str, pressed: bool) {
    let adapter = i_slint_core::window::WindowInner::from_pub(ui.window()).window_adapter();
    let text = i_slint_core::SharedString::from(text);
    i_slint_backend_testing::testing_backend::send_keyboard_key_text(&text, pressed, &adapter);
}

/// Send one key (press + release) to the window, like a real keystroke.
fn send(ui: &MainWindow, text: &str) {
    key_event(ui, text, true);
    key_event(ui, text, false);
}

/// A chord: hold Control, tap `key`, release Control. The modifier must
/// still be down when the key arrives — releasing Control first would make
/// a plain key event, which is exactly what the shortcut must not see.
fn ctrl_chord(ui: &MainWindow, key: &str) {
    key_event(ui, "\u{11}", true);
    key_event(ui, key, true);
    key_event(ui, key, false);
    key_event(ui, "\u{11}", false);
}

#[test]
fn ctrl_shortcuts_reach_the_bar_and_the_terminal() {
    i_slint_backend_testing::init_no_event_loop();

    let ui = MainWindow::new().expect("window under the testing backend");
    populate_slots(&ui, &["app-terminal".to_string()]);

    let query = Rc::new(RefCell::new(String::new()));
    let activated = Rc::new(RefCell::new(None::<String>));

    ui.on_query_changed({
        let query = query.clone();
        move |t| *query.borrow_mut() = t.to_string()
    });
    ui.on_slot_activated({
        let activated = activated.clone();
        move |id| *activated.borrow_mut() = Some(id.to_string())
    });

    // 1. Parked default focus: Ctrl+Space moves focus to the command bar,
    //    proven by typing reaching query-changed afterwards.
    ctrl_chord(&ui, " ");
    send(&ui, "n");
    send(&ui, "o");
    send(&ui, "t");
    send(&ui, "e");
    assert_eq!(
        query.borrow().as_str(),
        "note",
        "Ctrl+Space must focus the command bar from the default focus state"
    );

    // 2. Command bar focused: Ctrl+T bubbles past the input (which ignores
    //    modifier chords) and opens the Terminal slot.
    ctrl_chord(&ui, "t");
    assert_eq!(
        activated.borrow().as_deref(),
        Some("app-terminal"),
        "Ctrl+T must open the terminal while the command bar is focused"
    );

    // 3. Tab to a dock button: a modifier chord must bubble past it (not
    //    activate it), reach the window scope and focus the command bar.
    *activated.borrow_mut() = None;
    send(&ui, "\t");
    ctrl_chord(&ui, " ");
    send(&ui, "x");
    assert!(
        query.borrow().ends_with('x'),
        "Ctrl+Space on a focused button must bubble to the command bar, \
         not click the button (activated = {:?})",
        activated.borrow()
    );
    assert_eq!(
        activated.borrow().as_deref(),
        None,
        "a modifier chord must never activate a focused button"
    );

    // 4. A bare Space still activates the focused button (the guard only
    //    rejects modifier chords). Focus is on the command bar, so Tab
    //    walks to the dock first.
    send(&ui, "\t");
    send(&ui, " ");
    assert_eq!(
        activated.borrow().as_deref(),
        Some("app-terminal"),
        "plain Space must still activate the focused dock button"
    );
}
