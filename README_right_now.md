# Mandala Launcher (`mandala-shell`)

A non-attentional, privacy-first, tri-panel mobile shell written entirely in
**Rust** with a **Slint** declarative UI. Implements the architecture in
`comprehensive_minimalist_launcher_architecture.md`.

```
mandala-launcher/
├── Cargo.toml                    workspace
├── crates/
│   ├── mandala-module-api/       C-ABI contract + `declare_module!` macro
│   ├── mandala-core/             engine (no UI dependency)
│   │   ├── ipc_bus.rs            ACL-enforcing broker (Tokio mpsc)
│   │   ├── router.rs             async dispatcher -> modules -> replies
│   │   ├── loader.rs             SHA-256 + Ed25519 verified dlopen, monitor
│   │   ├── crypto.rs             digest/sign/verify, trust store
│   │   ├── state.rs              UDF reducer: Action -> Effect (pure)
│   │   ├── gesture.rs            thumb-zone gesture classifier (pure)
│   │   ├── command.rs            fuzzy search + command grammar
│   │   ├── slots.rs              the 15-slot A/B/C matrix
│   │   ├── theme.rs              JSON theme manifests + presets
│   │   └── paths.rs              on-disk layout
│   ├── mandala-ui/               Slint files + render/apply_theme glue
│   └── mandala-shell/            binary (desktop) / cdylib (Android)
├── modules/
│   ├── app-notes/                A2  notes (append-only text buffer)
│   ├── app-calculator/           C4  calculator (logs results to Notes over the IPC bus)
│   ├── app-terminal/             A1  sandboxed command shell (no process spawn)
│   ├── app-aegis/                C1  TOTP 2FA, imports otpauth:// URIs
│   ├── app-vault/                C2  Argon2id + XChaCha20-Poly1305 vault, KeePassDX CSV import
│   └── app-localsend/            C3  token-guarded LAN file transfer
├── tools/
│   ├── mandala-sign/             keygen / sign / verify / install
│   └── mandala-perf/             engine boot, RSS and reducer benchmarks
├── android/AndroidManifest.xml   HOME-category activity + launcher permissions
├── themes/                       Minimalist OLED, Cel-Shaded, Retro Terminal
└── scripts/
    ├── dev-setup.sh              build + key + sign + install, one command
    └── android-build.sh          cargo-ndk cross-build for aarch64 Android
```

## Quick start (desktop)

```bash
scripts/dev-setup.sh                       # builds, creates dev key, signs + installs modules
MANDALA_HOME=$PWD/mandala-home target/release/mandala-shell
```

Try in the command bar: `= 2*(3+4)^2`, `= save 19.99*3` (result is appended to
Notes through the ACL-checked bus), `note buy milk`, `calc` + Enter (fuzzy
launch), `theme next`, `monitor`.

Module command prefixes:

| Command | Module | Examples |
|---|---|---|
| `> …` | A1 Terminal | `> ls`, `> write todo.txt buy milk`, `> cat todo.txt`, `> date`, `> history` |
| `2fa …` | C1 2FA | `2fa`, `2fa add otpauth://totp/GitHub:me?secret=…`, `2fa code github` |
| `vault …` | C2 Vault | `vault init <pass>`, `vault unlock <pass>`, `vault add bank me pw`, `vault import export.csv` |
| `lsend …` | C3 LocalSend | `lsend recv 8080 token`, `lsend send /path/file 192.168.1.5:8080 token` |
| `weather …` | B1 Weather | `weather set https://your-host/obs`, `weather city Berlin`, `weather` |
| `rss …` | B2 RSS | `rss add https://example.org/feed.xml`, `rss fetch`, `rss headlines`, `rss read 2` |
| `tasks …` | B3 Tasks | `tasks add ship the launcher`, `tasks habit stretch`, `tasks done 1`, `tasks clean` |
| `doc …` | B4 Reader | `doc list`, `doc open README.md`, `doc next` |
| `gallery …` | B5 Gallery | `gallery set /home/me/Pictures`, `gallery list`, `gallery info cat.jpg` |
| `chat …` | A4 Chat | `chat token https://matrix.example.org me <token>`, `chat rooms`, `chat read 1`, `chat say 1 hello` |
| `ai …` / `ask …` | A5 Assistant | `ai set http://127.0.0.1:8080/v1`, `ai model llama3`, `ask summarise this` |
| `camera` | C5 Camera (Android) | fire the camera into `<home>/data/app-camera/captures`; `gallery set <that dir>` to browse |
| `wallpaper <path>` | Android only | `wallpaper /sdcard/Pictures/bg.png` |
| `overlay` | Android only | request the draw-over-other-apps permission |

Run the tests: `cargo test --workspace`.
Measure startup and memory: `cargo run --release -p mandala-perf`.
Headless module check (CI): `target/release/mandala-shell --selftest`.

## How the design maps to code

| Design section | Implementation |
|---|---|
| 1. Thumb ergonomics | Dock (Panel A) at the bottom; drawers B/C on the edges (`main_window.slint`) |
| 2. 15-slot matrix | `slots.rs`, rendered from three Slint models |
| 3. Gesture taxonomy | `gesture.rs`: lower-30% edge swipes open B/C, upper-70% edges are passed through to the OS, swipe-up minimises, double swipe-up / hold opens the process monitor |
| 4. Slint UI + theme tokens | `ui/*.slint`, `ThemePalette` global overwritten by `apply_theme` |
| 5. Microkernel IPC | `ipc_bus.rs` + `router.rs` |
| 6. Signed dynamic loading | `loader.rs` (flow chart implemented, minus remote fetch, see below) |
| 7. Responsive layout | `Responsive` global + `LayoutClass` (compact < 600, medium < 840, expanded: both drawers docked) |
| 8. Theme engine | `theme.rs`, `themes/*.json`; user themes in `<home>/themes/*.json` |
| 9. Roadmap phases | all five phases ticked in `docs/comprehensive_project_architecture_roadmap.md`; evidence in `docs/performance.md` and the test suite |

## Home directory layout

```
$MANDALA_HOME/
  modules/<id>.so  + <id>.so.sig     signed module + detached signature
  trust/*.pub                         trusted Ed25519 public keys (hex)
  themes/*.json                       user themes
  data/<id>/                          per-module private storage
  runtime/                            verified copies actually dlopen'ed (0700)
  quarantine/                         rejected binaries
```

## Security model (read this)

* **The signature is the trust boundary.** A module is an in-process shared
  library: once loaded it runs with the launcher's full privileges. The
  ACLs in `ipc_bus.rs` govern *message routing* between modules and the host;
  they are not memory or syscall isolation. Do not describe them as a sandbox.
* **The terminal does not spawn processes.** `> …` is a small command set over
  `<home>/data/app-terminal`; absolute paths, `..` and symlink escapes are
  rejected. Handing input to `sh -c` would give the command bar host
  privileges, so it is deliberately not implemented.
* **Vault data at rest** is Argon2id-derived (m=19 MiB, t=2, p=1) and sealed
  with XChaCha20-Poly1305; the passphrase is never stored. The passphrase is
  typed into the command bar, so it is visible on screen while typing.
* **2FA secrets** are base32 secrets in `<home>/data/app-aegis/accounts.json`
  (mode 0600) and are *not* encrypted at rest. The device lock is the boundary.
* **LocalSend** opens a TCP port only when the user runs `lsend recv`; the
  shared token is required, files are capped at 64 MiB and verified with
  SHA-256. Transfers are not encrypted in transit.
* Pipeline: read bytes → SHA-256 → `verify_strict` Ed25519 over the digest →
  on failure the file is moved to `quarantine/`. The **verified bytes** are
  written to `runtime/<id>-<hash>.so` and *that* copy is loaded, so swapping the
  file after verification does nothing (no TOCTOU).
* An empty trust store rejects every module. Keep `host.key` off the device;
  only `host.pub` goes into `trust/`.
* Module ids are restricted to `[a-z0-9_-]` (no path traversal).
* A module can only message targets its ACL allows (`AccessControlList::default_policy`:
  calculator → notes). Replies to the host are always allowed.
* Module identity in `host_send` is self-reported by the module; this is fine
  under the signed-code assumption, but is not a defence against hostile
  signed code.

## Writing a module

```rust
use mandala_module_api::{declare_module, Module, ModuleContext, ModuleReply};

struct Hello;
impl Module for Hello {
    fn id(&self) -> &'static str { "app-weather" }      // must equal the slot id
    fn name(&self) -> &'static str { "Weather" }
    fn init(&mut self, ctx: &ModuleContext) -> Result<(), String> { Ok(()) } // ctx.data_dir, ctx.host.send(..)
    fn handle(&mut self, action: &str, payload: &str) -> ModuleReply {
        ModuleReply::ok(format!("{action}: {payload}"))
    }
}
declare_module!(|| Hello);
```

Crate type `cdylib`; then
`mandala-sign install --home $MANDALA_HOME --key dev-keys/host.key target/release/libapp_weather.so`
(`libapp_weather.so` → id `app-weather`). Slot ids are listed in `slots.rs`.

## Android

The shell crate has an `android` feature and an `android_main` entry
(`crates/mandala-shell/src/lib.rs`), built as `libmandala_shell.so`. The
whole pipeline (shell, all fourteen modules, signing, staged home directory)
is one script:

```bash
cargo install cargo-ndk
rustup target add aarch64-linux-android
ANDROID_NDK_HOME=/path/to/ndk scripts/android-build.sh
```

It stages `dist/android/{AndroidManifest.xml,jniLibs/arm64-v8a/,mandala-home/,themes/}`.

**Packaging the APK** is `cargo apk2` (the Slint-maintained fork of `cargo-apk`;
plain `cargo-apk` has no 0.22 release and does not understand newer SDK
layouts). Its manifest comes from `[package.metadata.android]` in
`crates/mandala-shell/Cargo.toml` — the NativeActivity, `android.app.lib_name`,
HOME + LAUNCHER filters and the `SET_WALLPAPER` / `SYSTEM_ALERT_WINDOW` /
`INTERNET` permissions live there, so keep it in step with
`android/AndroidManifest.xml` (the Gradle-path template):

```bash
cargo install cargo-apk2 --version 1.5.0       # needs a JDK 17: d8 in
                                               # build-tools rejects newer class files
export ANDROID_HOME ANDROID_NDK_HOME
CARGO_APK_RELEASE_KEYSTORE=/path/release.jks \
CARGO_APK_RELEASE_KEYSTORE_PASSWORD=... \
  cargo apk2 build --release --features android \
    --target aarch64-linux-android -p mandala-shell
# -> target/release/apk/mandala.apk  (verify: apksigner verify --print-certs)
```

A release-profile build refuses to package without a keystore (only `dev`
auto-generates one), which is why CI takes `MANDALA_ANDROID_KEYSTORE` /
`MANDALA_ANDROID_KEYSTORE_PASSWORD` secrets and otherwise signs with a
throwaway key it prints a warning about. The Gradle path remains available and
is the one that puts `androidx.core` on the classpath for the FileProvider.

The JNI bridge in `crates/mandala-shell/src/android.rs` sets the wallpaper
(`WallpaperManager.setStream`), opens the overlay permission dialog
(`ACTION_MANAGE_OVERLAY_PERMISSION`), and fires the camera intent
(`ACTION_IMAGE_CAPTURE` with `EXTRA_OUTPUT`). It is compiled only for Android
targets. The `android` feature name `backend-android-activity-06` tracks
Slint's backend; adjust it if you pin a different Slint release.

**Camera on Android needs a FileProvider.** The camera app runs in another
process, so `EXTRA_OUTPUT` must be a `content://` URI. `android/AndroidManifest.xml`
declares `androidx.core.content.FileProvider` at `${applicationId}.fileprovider`
with paths from `android/res/xml/file_paths.xml`, which means the Gradle
project must put `androidx.core:core-ktx` (or `androidx.core:core`) on the
classpath. Without the provider the shell falls back to `Uri.fromFile`, which
Android rejects on API 24+ — the command then reports the rejection instead of
claiming the photo was saved.

## Status: what was and was not verified

Verified in this repository (Rust 1.98, Linux x86_64):

* `cargo build --workspace` succeeds, including the Slint build script.
* `cargo test --workspace`: **111 tests pass** (57 core, 9 terminal, 9 2FA,
  9 vault, 7 LocalSend, 7 calculator, 6 notes, 3 module API, 4 sign tool).
* `cargo clippy --workspace --all-targets`: no errors; only pre-existing style
  warnings in `app-calculator` and `mandala-shell` remain.
* `scripts/dev-setup.sh` builds, keygens, signs and installs all six modules.
* `mandala-shell --selftest` (headless) verifies + `dlopen`s every installed
  module and calls its `launch` action through the host, so the signature and
  ABI pipeline is exercised end to end.
* The shell starts against a real display. Release build: `boot: interactive
  after 0.85–1.28 s, process RSS 16.3–16.9 MiB` (three runs); debug build:
  `1.27 s, 36.9 MiB`. The headless engine alone is `0.29 ms / 3.5 MiB`
  (release). See [`docs/performance.md`](docs/performance.md) — the 20 ms /
  5 MB budgets are met by the engine, not by the Slint/winit GUI stack.
* The LocalSend module is exercised over real loopback TCP in its tests
  (handshake, token rejection, checksum rejection, file landed on disk).

Not verified here:

* **Android device run.** The `android` feature and `android.rs` JNI bridge are
  type-checked for `aarch64-linux-android` when that target is installed, and
  `scripts/android-build.sh` stages the APK inputs, but no NDK/device was
  available and the overlay/wallpaper calls were never executed.
* 2FA codes were validated against the RFC 6238 vectors, not against a live
  authenticator app.

## Not implemented (deliberately out of scope)

* **Remote fetch** of module artifacts from Git branches (design §6): the
  loader reads local, signed files. Add a downloader that drops files into
  `modules/` and let the existing verification do the rest.
* Android `.dex` classloading: modules are native shared objects.
* KeePass **KDBX** parsing (CSV import only) and the real LocalSend wire
  protocol; see the deviation table in the roadmap.
* Gesture "pass through to OS": the classifier reports
  `SystemBackPassthrough` and the UI takes no action; real pass-through is
  the OS's job when the app does not consume edge events.
