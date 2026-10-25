//! Mandala launcher entry points (desktop `main` and Android `android_main`).

#[cfg(target_os = "android")]
mod android;
mod app;

use anyhow::Context;
use app::App;
use mandala_core::crypto::TrustStore;
use mandala_core::ipc_bus::{AccessControlList, MicrokernelIpcBroker};
use mandala_core::loader::ModuleHost;
use mandala_core::paths::HostPaths;
use mandala_core::router::run_router;
use mandala_core::slots;
use mandala_core::state::Action;
use mandala_core::theme::ThemeManager;
use mandala_core::MandalaError;
use mandala_ui::MainWindow;
use slint::ComponentHandle;
use std::sync::Arc;

/// Build every subsystem, show the window and block until it closes.
pub fn run(paths: HostPaths) -> anyhow::Result<()> {
    run_inner(paths, false)
}

/// Like [`run`], but quits once the first frame is interactive and logs the
/// measured boot time + RSS. Used by `--boot-report` so the roadmap budget can
/// be checked on any machine (and in CI) without a human closing a window.
pub fn boot_report(paths: HostPaths) -> anyhow::Result<()> {
    run_inner(paths, true)
}

fn run_inner(paths: HostPaths, exit_after_boot: bool) -> anyhow::Result<()> {
    let boot = std::time::Instant::now();
    paths.ensure().context("creating Mandala home directory")?;
    let after_home = boot.elapsed();

    let trust = TrustStore::load_dir(&paths.trust_dir()).context("loading trusted keys")?;
    let no_keys = trust.is_empty();
    if no_keys {
        log::warn!(
            "no trusted public keys in {}: every module will be rejected",
            paths.trust_dir().display()
        );
    }

    let after_trust = boot.elapsed();
    let mut themes = ThemeManager::with_presets();
    for w in themes.load_user_dir(&paths.themes_dir()) {
        log::warn!("theme skipped: {w}");
    }
    let after_themes = boot.elapsed();

    // One worker drives the router task (it is single-threaded by design) and
    // module calls go to the blocking pool via `spawn_blocking`. A second
    // worker only added a stack + wakeups to boot and idle RSS.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .context("starting tokio runtime")?;

    let after_runtime = boot.elapsed();
    let (broker, rx) = MicrokernelIpcBroker::new(64, AccessControlList::default_policy());
    let broker = Arc::new(broker);
    let host = Arc::new(ModuleHost::new(paths.clone(), trust, broker.clone()));
    let after_host = boot.elapsed();

    let ui = MainWindow::new().context("creating window")?;
    let after_window = boot.elapsed();
    mandala_ui::populate_slots(&ui);

    let app = App::new(
        broker.clone(),
        host.clone(),
        themes,
        runtime.handle().clone(),
        ui.as_weak(),
    );
    app.apply_current_theme(&ui);

    // Messages addressed to the host flow back into the reducer.
    {
        let app = app.clone();
        runtime.spawn(run_router(rx, broker, host, move |env| {
            app.on_host_envelope(env)
        }));
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

    let after_wiring = boot.elapsed();

    // Roadmap Phase 5 budget: report the real time to an interactive event
    // loop and the resident set, instead of asserting a target.
    slint::Timer::single_shot(std::time::Duration::ZERO, move || {
        let rss = mandala_core::sys::process_rss_kb()
            .map(|k| format!(", process RSS {k} KiB"))
            .unwrap_or_default();
        let ms = |d: std::time::Duration| d.as_secs_f64() * 1e3;
        log::info!(
            "boot: interactive after {:.1} ms{rss}\n  phases: home {:.1} · trust {:.1} · themes {:.1} · runtime {:.1} · host {:.1} · window {:.1} · wiring {:.1} · event-loop start {:.1} ms",
            ms(boot.elapsed()),
            ms(after_home),
            ms(after_trust - after_home),
            ms(after_themes - after_trust),
            ms(after_runtime - after_themes),
            ms(after_host - after_runtime),
            ms(after_window - after_host),
            ms(after_wiring - after_window),
            ms(boot.elapsed() - after_wiring),
        );
        if exit_after_boot {
            let _ = slint::quit_event_loop();
        }
    });

    ui.run().context("running UI event loop")?;
    // Dropping the runtime stops the router; unload modules cleanly first.
    host_shutdown(&app);
    Ok(())
}

fn host_shutdown(_app: &Arc<App>) {
    // Modules are unloaded when `ModuleHost` is dropped with the last Arc.
    log::info!("shutting down");
}

/// Headless smoke test (`mandala-shell --selftest`): verify, load and call
/// every installed slot module. Exits non-zero if any module fails, so CI can
/// catch a broken signature pipeline or a module that no longer loads.
pub fn selftest(paths: HostPaths) -> anyhow::Result<()> {
    paths.ensure().context("creating Mandala home directory")?;
    let trust = TrustStore::load_dir(&paths.trust_dir()).context("loading trusted keys")?;
    let (broker, _rx) = MicrokernelIpcBroker::new(8, AccessControlList::default_policy());
    let host = ModuleHost::new(paths, trust, Arc::new(broker));

    let (mut loaded, mut skipped, mut failures) = (0usize, 0usize, 0usize);
    for id in slots::module_ids() {
        match host.call(id, "launch", "") {
            Ok(reply) if reply.ok => {
                loaded += 1;
                let first = reply.text.lines().next().unwrap_or("").trim();
                println!("ok   {id:<16} {first}");
            }
            Ok(reply) => {
                failures += 1;
                println!("FAIL {id:<16} {}", reply.text);
            }
            // An empty slot is a supported state: slots render as "not
            // installed" until a signed module is placed in modules/.
            Err(MandalaError::NotInstalled(_)) => {
                skipped += 1;
                println!("skip {id:<16} not installed");
            }
            Err(e) => {
                failures += 1;
                println!("FAIL {id:<16} {e}");
            }
        }
    }
    if failures > 0 {
        anyhow::bail!(
            "{failures} of {} modules failed",
            slots::module_ids().count()
        );
    }
    println!("selftest passed: {loaded} modules loaded, {skipped} slots without a module");
    Ok(())
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
