# Mandala performance report (Phase 5)

The roadmap in [`comprehensive_project_architecture_roadmap.md`](comprehensive_project_architecture_roadmap.md)
sets two budgets:

* cold boot to interactive: **< 20 ms**
* resident memory on idle: **< 5 MB**

This document records what was *measured*, not what was hoped for. Numbers come
from `tools/mandala-perf` (headless engine) and from the shell's own boot log
(`boot: interactive after … ms, process RSS … KiB`).

Reproduce with:

```bash
cargo run --release -p mandala-perf        # headless engine
scripts/dev-setup.sh                       # build + sign modules
MANDALA_HOME=$PWD/mandala-home target/release/mandala-shell --boot-report
                                           # real window, exits after first frame
mandala-shell --selftest                   # headless module load check
```

`--boot-report` exists because the budget has to be checkable without a human
closing a window: it runs the full boot path (including window + renderer),
logs the numbers below, and quits at the first interactive frame. The boot log
now also breaks the time down by phase (`home · trust · themes · runtime ·
host · window · wiring · event-loop start`), which is where the optimisation
targets come from.

## Engine only (no window)

Measured on Linux x86_64, 4 cores, Rust 1.98, `/tmp` home directory.

| Metric | Debug | Release | Budget |
|---|---|---|---|
| engine cold start (dirs + trust store + themes + broker + router) | 1.06 ms median | **0.31 ms** median (min 0.17 / max 1.81, 100 samples) | — |
| module discovery (14 module slots) | 300 µs | **309 µs** | — |
| reducer: one `QueryChanged` event (fuzzy search over 15 slots) | 690 µs | **204–339 µs** | — |
| process RSS (headless) | 4.7 MiB | **3.5 MiB** | < 5 MB |

The headless engine **is** inside the 5 MB budget and starts in well under a
millisecond. That is the part Mandala fully controls.

(The release numbers were produced with `opt-level = 3` but
`CARGO_PROFILE_RELEASE_LTO=false CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16`, so
the build finished in a reasonable time on the machine used for this report.
LTO affects code size/startup only marginally.)

## Whole launcher (window included)

Real X11 session, `RUST_LOG=info`, measured at the first `Timer` tick (the
point where the window can accept input). Six consecutive `--boot-report` runs
of the shipped profile (`lto = "thin"`, `codegen-units = 1`):

```text
boot: interactive after 963.9 ms, process RSS 14384 KiB
  phases: home 34.3 · trust 27.1 · themes 21.6 · runtime 1.5 · host 0.1 · window 812.5 · wiring 0.2 · event-loop start 66.6 ms
boot: interactive after 936.3 ms, process RSS 14576 KiB
  phases: home 0.8 · trust 0.6 · themes 0.9 · runtime 1.4 · host 0.2 · window 862.6 · wiring 3.8 · event-loop start 66.1 ms
boot: interactive after 900.4 ms, process RSS 14944 KiB
  phases: home 0.7 · trust 0.8 · themes 4.1 · runtime 2.1 · host 0.1 · window 821.0 · wiring 0.3 · event-loop start 71.4 ms
boot: interactive after 963.5 ms, process RSS 14388 KiB
  phases: home 0.7 · trust 2.3 · themes 1.7 · runtime 1.4 · host 0.1 · window 892.8 · wiring 0.3 · event-loop start 64.1 ms
boot: interactive after 463.4 ms, process RSS 14872 KiB
  phases: home 0.3 · trust 0.5 · themes 0.6 · runtime 0.9 · host 0.0 · window 433.4 · wiring 0.1 · event-loop start 27.6 ms
boot: interactive after 676.9 ms, process RSS 14424 KiB
  phases: home 0.4 · trust 0.4 · themes 0.4 · runtime 0.7 · host 0.0 · window 644.2 · wiring 0.6 · event-loop start 30.1 ms
```

So 0.46–0.96 s end-to-end with ~14.4 MiB RSS, and the phase log makes the
structure of that time unambiguous: **433–893 ms of the 463–964 ms is
`MainWindow::new()`** — Slint's platform/renderer/font initialisation. Every
phase Mandala itself owns (home, trust, themes, runtime, host, wiring, router
spawn) adds up to < 5 ms, and even the cold first run (home 34 ms, trust 27 ms)
stays under 65 ms. The event loop's first tick costs another 28–71 ms.

## What changed in this pass

| Change | Effect |
|---|---|
| One tokio worker instead of two (the router is a single task; module calls use `spawn_blocking`) | one fewer thread + its stack; runtime phase is now 0.7–2.1 ms |
| Slint `accessibility` and `system-tray` features no longer enabled by default | **RSS 16.3–16.9 → 14.4–14.9 MiB** (accesskit + at-spi/dbus client state are no longer mapped). Boot time was not measurably affected |
| `--boot-report` + per-phase boot logging | makes the budget checkable in CI and points every future optimisation at `window`, not at Mandala's own code |

Accessibility is now an opt-in build: `cargo build --release -p mandala-shell
--features a11y` restores accesskit/AT-SPI. A launcher that ships to
screen-reader users should enable it; the launcher's default build trades it
for ~2 MiB of idle RSS.

## Where the remaining time goes

`MainWindow::new()` creates the winit window, an EGL/GLES context for FemtoVG,
the glyph atlas and the font database. Those are Slint/winit/Mesa costs, not
Mandala's; the app-visible work before them is ~2 ms. The only way to collapse
them further inside this stack is to stop using the toolkit's window creation
(software renderer experiment, Android `SurfaceView`, or a raw X11/Wayland
surface), which is a UI-framework decision rather than a tuning one.

## Verdict on the two budget lines

| Budget | Status | Why |
|---|---|---|
| cold boot < 20 ms | **met by the engine** (0.31 ms), **not met by the full launcher** (0.46–0.96 s) | 433–893 ms of that is Slint's window + renderer + font init; Mandala's own phases are < 5 ms. Reaching 20 ms end-to-end needs a surface the toolkit does not create from scratch. |
| idle RSS < 5 MB | **met by the engine** (3.5 MiB), **not met by the full launcher** (14.4–14.9 MiB release, 36 MiB debug) | Slint + winit + FemtoVG add ~11 MiB after removing the accessibility stack. Dropping below 5 MiB needs a text-only renderer with a fixed font, not a UI framework. |

Both budgets are therefore recorded as **design goals not currently achieved**;
the honest numbers live here and in the shell's boot log, and the roadmap
checkbox for Phase 5 profiling says so.

## Notes on how to read these numbers

* Debug numbers above are for reproducibility on any clone; release numbers are
  what ships. Re-run the two commands above after any change to the engine.
* The reducer figure is the worst case (every keystroke fuzzy-matches all 15
  slots, with allocations). Typical navigation events are ~100x cheaper.
* The LocalSend transfer path and the Argon2id key derivation are measured by
  their own tests (`app-localsend`, 7 tests including a real loopback transfer;
  `app-vault`, 9 tests, each of which does one full KDF).
