//! Keyboard focus loop: Tab must walk the command bar and the controls
//! behind it, Enter must activate the focused control, and the default
//! focus state must keep handling Escape/back.
//!
//! Runs the real `MainWindow` on Slint's headless testing backend, so no
//! display or window system is involved.

use mandala_ui::{populate_slots, MainWindow};
use slint::ComponentHandle;
use std::cell::RefCell;
use std::rc::Rc;

/// Send one key (press + release) to the window, like a real keystroke.
/// Raw `testing_backend` instead of the component-level helper because the
/// crate's `internal` feature does not build outside Slint's own repo.
fn send(ui: &MainWindow, text: &str) {
    let adapter = i_slint_core::window::WindowInner::from_pub(ui.window()).window_adapter();
    let text = i_slint_core::SharedString::from(text);
    i_slint_backend_testing::testing_backend::send_keyboard_key_text(&text, true, &adapter);
    i_slint_backend_testing::testing_backend::send_keyboard_key_text(&text, false, &adapter);
}

#[test]
fn tab_walks_command_bar_then_buttons() {
    i_slint_backend_testing::init_no_event_loop();

    let ui = MainWindow::new().expect("window under the testing backend");
    // Install the first dock slot so it activates itself instead of the
    // placeholder (which would open the Store instead).
    populate_slots(&ui, &["app-terminal".to_string()]);

    let back = Rc::new(RefCell::new(0u32));
    let query = Rc::new(RefCell::new(String::new()));
    let activated = Rc::new(RefCell::new(None::<String>));

    ui.on_back({
        let back = back.clone();
        move || *back.borrow_mut() += 1
    });
    ui.on_query_changed({
        let query = query.clone();
        move |t| *query.borrow_mut() = t.to_string()
    });
    ui.on_slot_activated({
        let activated = activated.clone();
        move |id| *activated.borrow_mut() = Some(id.to_string())
    });

    // 1. Initial focus parks on the back scope: Escape reaches back().
    send(&ui, "\u{1b}");
    assert_eq!(
        *back.borrow(),
        1,
        "Escape must fire back() from the default focus state"
    );

    // 2. Tab #1 skips the invisible root scopes and lands on the command
    //    bar: typing is reported through query-changed.
    send(&ui, "\t");
    send(&ui, "n");
    send(&ui, "o");
    send(&ui, "t");
    send(&ui, "e");
    assert_eq!(
        query.borrow().as_str(),
        "note",
        "Tab must focus the command bar so it receives typing"
    );

    // 3. Tab #2 walks on to the first dock button; Enter activates it.
    send(&ui, "\t");
    send(&ui, "\n");
    assert_eq!(
        activated.borrow().as_deref(),
        Some("app-terminal"),
        "Tab must reach the dock buttons and Enter must activate the focused one"
    );
}
