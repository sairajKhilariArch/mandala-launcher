//! Mandala launcher entry points (desktop `main` and Android `android_main`).

mod app;

use anyhow::Context;
use app::App;
use mandala_core::crypto::TrustStore;
use mandala_core::ipc_bus::{AccessControlList, MicrokernelIpcBroker};
use mandala_core::loader::ModuleHost;
use mandala_core::paths::HostPaths;
use mandala_core::router::run_router;
use mandala_core::state::Action;
use mandala_core::theme::ThemeManager;
use mandala_ui::MainWindow;
use slint::ComponentHandle;
use std::sync::Arc;

/// Build every subsystem, show the window and block until it closes.
pub fn run(paths: HostPaths) -> anyhow::Result<()> {
    paths.ensure().context("creating Mandala home directory")?;

    let trust = TrustStore::load_dir(&paths.trust_dir()).context("loading trusted keys")?;
    let no_keys = trust.is_empty();
    if no_keys {
        log::warn!("no trusted public keys in {}: every module will be rejected", paths.trust_dir().display());
    }

    let mut themes = ThemeManager::with_presets();
    for w in themes.load_user_dir(&paths.themes_dir()) {
        log::warn!("theme skipped: {w}");
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .context("starting tokio runtime")?;

    let (broker, rx) = MicrokernelIpcBroker::new(64, AccessControlList::default_policy());
    let broker = Arc::new(broker);
    let host = Arc::new(ModuleHost::new(paths.clone(), trust, broker.clone()));

    let ui = MainWindow::new().context("creating window")?;
    mandala_ui::populate_slots(&ui);

    let app = App::new(broker.clone(), host.clone(), themes, runtime.handle().clone(), ui.as_weak());
    app.apply_current_theme(&ui);

    // Messages addressed to the host flow back into the reducer.
    {
        let app = app.clone();
        runtime.spawn(run_router(rx, broker, host, move |env| app.on_host_envelope(env)));
    }

    ui.on_slot_activated({
        let app = app.clone();
        move |id| app.dispatch(Action::OpenSlot(id.to_string()))
    });
    ui.on_query_changed({
        let app = app.clone();
        move |q| app.dispatch(Action::QueryChanged(q.to_string()))
    });
    ui.on_query_submitted({
        let app = app.clone();
        move || app.dispatch(Action::QuerySubmitted)
    });
    ui.on_close_overlays({
        let app = app.clone();
        move || app.dispatch(Action::CloseOverlays)
    });
    ui.on_resized({
        let app = app.clone();
        move |w| app.dispatch(Action::Resize(w))
    });
    ui.on_pointer_down({
        let app = app.clone();
        move |x, y| app.on_pointer_down(x, y)
    });
    ui.on_pointer_up({
        let app = app.clone();
        move |x, y, w, h| app.on_pointer_up(x, y, w, h)
    });
    ui.on_monitor_unload_all({
        let app = app.clone();
        move || app.dispatch(Action::MonitorUnloadAll)
    });
    ui.on_monitor_restart_all({
        let app = app.clone();
        move || app.dispatch(Action::MonitorRestartAll)
    });

    if no_keys {
        app.dispatch(Action::Notice(
            "No trusted keys installed, so no module can load.\nRun scripts/dev-setup.sh (see README).".into(),
        ));
    }

    ui.run().context("running UI event loop")?;
    // Dropping the runtime stops the router; unload modules cleanly first.
    host_shutdown(&app);
    Ok(())
}

fn host_shutdown(_app: &Arc<App>) {
    // Modules are unloaded when `ModuleHost` is dropped with the last Arc.
    log::info!("shutting down");
}

#[cfg(all(target_os = "android", feature = "android"))]
#[no_mangle]
fn android_main(app: slint::android::AndroidApp) {
    slint::android::init(app.clone()).expect("slint android init");
    let root = app
        .internal_data_path()
        .unwrap_or_else(|| std::path::PathBuf::from("/data/local/tmp/mandala"));
    if let Err(e) = run(HostPaths::new(root)) {
        log::error!("mandala exited with error: {e:#}");
    }
}
