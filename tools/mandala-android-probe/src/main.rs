//! `mandala-android-probe`: exercises the module pipeline on a real Android
//! runtime (emulator or device) without needing an APK or an Activity.
//!
//! The GUI shell is a `cdylib` that Android can only load through an APK +
//! `NativeActivity`, but the *security and ABI pipeline* is ordinary Rust:
//! HostPaths -> TrustStore -> ModuleHost -> dlopen -> `handle("launch")`.
//! This probe runs exactly that, for every slot, and exits non-zero on the
//! first module that fails to verify or load. That makes it a real device
//! check of the cross-compiled modules, the Ed25519 signatures and the C ABI.
//!
//! The JNI calls in `crates/mandala-shell/src/android.rs` (wallpaper, overlay,
//! camera) need an Activity + JavaVM and are therefore only reachable from the
//! APK build; this probe reports that limitation instead of pretending.
//!
//! ```text
//! adb push dist/android/mandala-home /data/local/tmp/
//! adb push mandala-android-probe /data/local/tmp/
//! adb shell chmod +x /data/local/tmp/mandala-android-probe
//! adb shell MANDALA_HOME=/data/local/tmp/mandala-home /data/local/tmp/mandala-android-probe
//! ```

use mandala_core::crypto::TrustStore;
use mandala_core::ipc_bus::{AccessControlList, MicrokernelIpcBroker};
use mandala_core::loader::ModuleHost;
use mandala_core::paths::HostPaths;
use mandala_core::slots;
use mandala_core::MandalaError;
use std::sync::Arc;

fn main() {
    let root = std::env::var_os("MANDALA_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("./mandala-home"));
    println!("mandala-android-probe");
    println!("  home   {}", root.display());
    println!(
        "  target {} / {}",
        std::env::consts::ARCH,
        std::env::consts::OS
    );

    let paths = HostPaths::new(root);
    if let Err(e) = paths.ensure() {
        eprintln!("FAIL home: {e}");
        std::process::exit(1);
    }
    let trust = match TrustStore::load_dir(&paths.trust_dir()) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("FAIL trust store: {e}");
            std::process::exit(1);
        }
    };
    println!("  trusted keys {}", trust.len());
    if trust.is_empty() {
        eprintln!(
            "FAIL no trusted keys in {}: every module would be rejected",
            paths.trust_dir().display()
        );
        std::process::exit(1);
    }

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
                println!("FAIL {id:<16} {}", reply.text.replace('\n', " | "));
            }
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

    println!();
    println!("probe result: {loaded} loaded, {skipped} not installed, {failures} failed");
    println!("note: wallpaper/overlay/camera JNI need an Activity and are only reachable from the APK build.");
    if failures > 0 {
        std::process::exit(1);
    }
}
