//! `mandala-perf`: measures the parts of the roadmap performance budget that
//! are measurable without a display server.
//!
//! ```text
//! mandala-perf                 # 50 iterations in a temporary home
//! mandala-perf --iters 200     # more samples
//! mandala-perf --home ./mandala-home
//! ```
//!
//! Measured here: engine cold start (directories, trust store, themes, broker,
//! router task), module discovery, reducer throughput, and process RSS.
//! Window creation and first paint need a display and are logged by the shell
//! itself (`boot: interactive after …`).

use mandala_core::crypto::TrustStore;
use mandala_core::ipc_bus::{AccessControlList, MicrokernelIpcBroker};
use mandala_core::loader::ModuleHost;
use mandala_core::paths::HostPaths;
use mandala_core::router::run_router;
use mandala_core::slots;
use mandala_core::state::{reduce, Action, AppState};
use mandala_core::sys;
use mandala_core::theme::ThemeManager;
use std::sync::Arc;
use std::time::{Duration, Instant};

const USAGE: &str = "usage: mandala-perf [--iters N] [--home DIR] [--json]";

struct Args {
    iters: usize,
    home: Option<String>,
    json: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        iters: 50,
        home: None,
        json: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--iters" => {
                let v = it.next().ok_or("--iters needs a value")?;
                args.iters = v.parse().map_err(|_| format!("bad --iters value `{v}`"))?;
                if args.iters == 0 {
                    return Err("--iters must be at least 1".into());
                }
            }
            "--home" => args.home = Some(it.next().ok_or("--home needs a value")?),
            "--json" => args.json = true,
            "-h" | "--help" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument `{other}`\n{USAGE}")),
        }
    }
    Ok(args)
}

/// One engine cold start, exactly as `run()` assembles it (minus the window).
fn cold_start(paths: &HostPaths) -> Result<(), String> {
    paths.ensure().map_err(|e| e.to_string())?;
    let trust = TrustStore::load_dir(&paths.trust_dir()).map_err(|e| e.to_string())?;
    let mut themes = ThemeManager::with_presets();
    let _ = themes.load_user_dir(&paths.themes_dir());
    let (broker, rx) = MicrokernelIpcBroker::new(64, AccessControlList::default_policy());
    let broker = Arc::new(broker);
    let host = Arc::new(ModuleHost::new(paths.clone(), trust, broker.clone()));

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    let handle = runtime.spawn(run_router(rx, broker, host, |_| {}));
    // Let the router reach its first await, then stop it. Spawning without
    // polling would leave the task queue populated at runtime shutdown.
    runtime.block_on(async { tokio::task::yield_now().await });
    handle.abort();
    Ok(())
}

fn median(mut v: Vec<Duration>) -> Duration {
    v.sort();
    v[v.len() / 2]
}

fn main() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };

    let (paths, _tmp) = match &args.home {
        Some(h) => (HostPaths::new(h), None),
        None => {
            let dir = std::env::temp_dir().join(format!("mandala-perf-{}", std::process::id()));
            (HostPaths::new(&dir), Some(dir))
        }
    };

    // Warm the page cache / allocator once so the first sample is not an outlier.
    let _ = cold_start(&paths);

    let mut samples = Vec::with_capacity(args.iters);
    for _ in 0..args.iters {
        let t = Instant::now();
        if let Err(e) = cold_start(&paths) {
            eprintln!("cold start failed: {e}");
            std::process::exit(1);
        }
        samples.push(t.elapsed());
    }

    // Module discovery for all 15 slots.
    let trust = TrustStore::load_dir(&paths.trust_dir()).unwrap_or_default();
    let (broker, _rx) = MicrokernelIpcBroker::new(8, AccessControlList::default_policy());
    let host = ModuleHost::new(paths.clone(), trust, Arc::new(broker));
    let t = Instant::now();
    let statuses = host.status(slots::module_ids());
    let discovery = t.elapsed();

    // Reducer throughput: 20k input events through the pure state machine.
    let mut state = AppState::default();
    let t = Instant::now();
    const N: u32 = 20_000;
    for i in 0..N {
        reduce(&mut state, Action::QueryChanged(format!("calc{i}")));
    }
    let reduce_elapsed = t.elapsed();

    let rss = sys::process_rss_kb();
    let peak = sys::process_peak_rss_kb();
    let min = *samples.iter().min().unwrap();
    let med = median(samples.clone());
    let max = *samples.iter().max().unwrap();
    let mean = samples.iter().sum::<Duration>() / samples.len() as u32;

    if args.json {
        println!(
            "{{\"iters\":{},\"engine_boot_ms\":{{\"min\":{:.3},\"median\":{:.3},\"mean\":{:.3},\"max\":{:.3}}},\"module_discovery_us\":{},\"reduce_per_event_us\":{:.3},\"rss_kb\":{},\"peak_rss_kb\":{}}}",
            args.iters,
            min.as_secs_f64() * 1e3,
            med.as_secs_f64() * 1e3,
            mean.as_secs_f64() * 1e3,
            max.as_secs_f64() * 1e3,
            discovery.as_micros(),
            reduce_elapsed.as_secs_f64() * 1e6 / N as f64,
            rss.map(|v| v.to_string()).unwrap_or_else(|| "null".into()),
            peak.map(|v| v.to_string()).unwrap_or_else(|| "null".into()),
        );
    } else {
        println!("MANDALA PERFORMANCE REPORT");
        println!("  home                {}", paths.root.display());
        println!("  iterations          {}", args.iters);
        println!();
        println!(
            "  engine cold start   min {:.3} ms | median {:.3} ms | mean {:.3} ms | max {:.3} ms",
            ms(min),
            ms(med),
            ms(mean),
            ms(max)
        );
        println!(
            "  module discovery    {} us ({} slots)",
            discovery.as_micros(),
            statuses.len()
        );
        println!(
            "  reducer throughput  {:.3} us/event ({:.0}k events/s)",
            reduce_elapsed.as_secs_f64() * 1e6 / N as f64,
            N as f64 / reduce_elapsed.as_secs_f64() / 1000.0
        );
        match (rss, peak) {
            (Some(r), p) => println!(
                "  process RSS         {} KiB (peak {})",
                r,
                p.map(|v| format!("{v} KiB"))
                    .unwrap_or_else(|| "n/a".into())
            ),
            (None, _) => println!("  process RSS         not exposed by this OS"),
        }
        println!();
        println!("  roadmap budget: cold boot < 20 ms, idle RSS < 5 MB");
        println!("  note: RSS here is the headless engine. The window (Slint + winit)");
        println!("        adds its own footprint, which the shell logs at boot.");
    }

    if let Some(dir) = _tmp {
        let _ = std::fs::remove_dir_all(dir);
    }
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1e3
}
