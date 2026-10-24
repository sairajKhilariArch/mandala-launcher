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
MANDALA_HOME=$PWD/mandala-home RUST_LOG=info target/release/mandala-shell
mandala-shell --selftest                   # headless module load check
```

## Engine only (no window)

Measured on Linux x86_64, 4 cores, Rust 1.98, `/tmp` home directory.

| Metric | Debug | Release | Budget |
|---|---|---|---|
| engine cold start (dirs + trust store + themes + broker + router) | 1.06 ms median | **0.292 ms** median (min 0.165 / max 0.491, 30 samples) | — |
| module discovery (14 module slots) | 300 µs | **283 µs** | — |
| reducer: one `QueryChanged` event (fuzzy search over 15 slots) | 690 µs | **183 µs** | — |
| process RSS (headless) | 4.7 MiB | **3.5 MiB** | < 5 MB |

The headless engine **is** inside the 5 MB budget and starts in well under a
millisecond. That is the part Mandala fully controls.

(The release numbers were produced with `opt-level = 3` but
`CARGO_PROFILE_RELEASE_LTO=false CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16`, so
the build finished in a reasonable time on the machine used for this report.
LTO affects code size/startup only marginally.)

## Whole launcher (window included)

Real Wayland/X11 session, `RUST_LOG=info`, measured at the first `Timer` tick
(the point where the window can accept input):

```text
debug   boot: interactive after 1272.8 ms, process RSS 36964 KiB
release boot: interactive after  852.7 ms, process RSS 16908 KiB
release boot: interactive after 1065.0 ms, process RSS 16768 KiB
release boot: interactive after 1277.5 ms, process RSS 16568 KiB
```

Three consecutive release runs span 0.85–1.28 s (the machine's load average
was 6–8 during the measurement), and RSS settles at ~16.3–16.5 MiB. So the GUI
stack — Slint, winit, the FemtoVG renderer, font loading, clipboard/portal
integration — dominates both numbers: the window costs ~0.9–1.3 s and ~13 MiB
on top of the ~0.3 ms / 3.5 MiB engine. First paint follows within a frame of
the `boot:` log line.

## Verdict on the two budget lines

| Budget | Status | Why |
|---|---|---|
| cold boot < 20 ms | **met by the engine** (0.29 ms), **not met by the full launcher** (0.85–1.28 s) | Window creation, renderer init and font database construction dominate; they are not avoided by Rust. Reaching 20 ms end-to-end means a Wayland/X11-independent surface (or an Android `SurfaceView` path) rather than winit. |
| idle RSS < 5 MB | **met by the engine** (3.5 MiB), **not met by the full launcher** (16.3 MiB release, 36 MiB debug) | Slint + winit + FemtoVG add ~13 MiB (glyph atlas, renderer state, winit/portal state). Dropping below 5 MiB needs a text-only renderer (e.g. tiny-skia software with a fixed font), not a UI framework. |

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
