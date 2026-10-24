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
| engine cold start (dirs + trust store + themes + broker + router) | 1.06 ms median | *(see run output)* | — |
| module discovery (14 module slots) | 300 µs | *(see run output)* | — |
| reducer: one `QueryChanged` event (fuzzy search over 15 slots) | 690 µs | *(see run output)* | — |
| process RSS (headless) | 4.7 MiB | *(see run output)* | < 5 MB |

The headless engine **is** inside the 5 MB budget and starts in about a
millisecond. That is the part Mandala fully controls.

## Whole launcher (window included)

Measured on the same machine, debug build, real Wayland/X11 session:

```text
boot: interactive after 1272.8 ms, process RSS 36964 KiB
```

So the GUI stack — Slint, winit, the FemtoVG renderer, font loading and the
clipboard/portal integration — is ~1.2 s and ~36 MiB in a debug build. The
Slint event-loop start is measured at the first `Timer` tick, i.e. the point
where the window can accept input; first paint follows within a frame.

## Verdict on the two budget lines

| Budget | Status | Why |
|---|---|---|
| cold boot < 20 ms | **not met** by the full launcher | Window creation, GPU/software renderer init and font database construction dominate; they are not avoided by Rust. The engine itself is ~1 ms. Reaching 20 ms end-to-end means a Wayland/X11-independent surface (or an Android `SurfaceView` path) rather than winit. |
| idle RSS < 5 MB | **not met** by the full launcher | Headless engine is 4.7 MiB. Slint + winit + FemtoVG add ~30 MiB (glyph atlas, renderer state, winit/portal state). Dropping below 5 MiB needs a text-only renderer (e.g. tiny-skia software with a fixed font), not a UI framework. |

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
