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
│   ├── app-notes/                A2  (example dynamic module)
│   └── app-calculator/           C4  (example; talks to Notes over the IPC bus)
├── tools/mandala-sign/           keygen / sign / verify / install
├── themes/                       Minimalist OLED, Cel-Shaded, Retro Terminal
└── scripts/dev-setup.sh          build + key + sign + install, one command
```

## Quick start (desktop)

```bash
scripts/dev-setup.sh                       # builds, creates dev key, signs + installs modules
MANDALA_HOME=$PWD/mandala-home target/release/mandala-shell
```

Try in the command bar: `= 2*(3+4)^2`, `= save 19.99*3` (result is appended to
Notes through the ACL-checked bus), `note buy milk`, `calc` + Enter (fuzzy
launch), `theme next`, `monitor`.

Run the tests: `cargo test --workspace`.

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
(`crates/mandala-shell/src/lib.rs`), built as `libmandala_shell.so`:

```bash
cargo install cargo-ndk
cargo ndk -t arm64-v8a build --release -p mandala-shell --lib --features android
```

You still need to package it (e.g. with `cargo-apk`/`xbuild` or a Gradle
shell), declare a HOME-category activity in the manifest, and sign module
`.so` files for the Android ABI. The feature name `backend-android-activity-06`
in `crates/mandala-shell/Cargo.toml` tracks Slint's Android backend, so adjust
it if you pin a different Slint release.

## Status: what was and was not verified

Built in an environment **without access to crates.io**, so `cargo build` /
`cargo test` could not be run for the whole workspace. What *was* verified by
compiling directly with `rustc`:

* `gesture`, `slots`, `command`, `state`: 31 unit tests pass.
* `app-calculator` (7 tests) and `app-notes` (6 tests) pass.
* The real `declare_module!` output was compiled into `cdylib`s, `dlopen`ed,
  and driven through the vtable (init, handle, free_string, and a calculator
  `save` going out through the host callback to Notes).

**Not yet compiled** (they need the external crates): `ipc_bus`, `crypto`,
`loader`, `router`, `theme`, `mandala-sign` (serde, tokio, ed25519-dalek,
libloading, sha2), the Slint files, and the shell. They were written
carefully and include tests, but expect to fix a few compile errors on the
first `cargo build`, most likely in the Slint files (property/callback syntax
varies a little between Slint releases) or small API details. The first
build also generates `Cargo.lock`.

## Not implemented (deliberately out of scope)

* **Remote fetch** of module artifacts from Git branches (design §6): the
  loader reads local, signed files. Add a downloader that drops files into
  `modules/` and let the existing verification do the rest.
* Android `.dex` classloading: modules are native shared objects.
* The 13 other slot modules (terminal, chat, AI, weather, ...). Slots show
  "not installed" until a signed module with the matching id is placed in
  `modules/`.
* Gesture "pass through to OS": the classifier reports
  `SystemBackPassthrough` and the UI takes no action; real pass-through is
  the OS's job when the app does not consume edge events.
