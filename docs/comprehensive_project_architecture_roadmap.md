# Mandala Launcher (`mandala-shell`)
## Comprehensive Technical Architecture, Ergonomic Specifications, & Engineering Roadmap

**Document Version:** 1.0.0  
**Target Platform:** Android (ARM64 / `aarch64-linux-android`) & Linux Desktop (`x86_64-unknown-linux-gnu`)  
**Core Technologies:** Rust (`std` + Tokio Async) | Slint Native Declarative UI Engine | Ed25519 Microkernel Security Bus

---

## Table of Contents
1. [Executive Summary & Vision](#1-executive-summary--vision)
2. [Cognitive Ergonomics & Behavioral Framework](#2-cognitive-ergonomics--behavioral-framework)
3. [Spatial Architecture & 15-Slot Matrix](#3-spatial-architecture--15-slot-matrix)
4. [System Architecture & Technology Stack](#4-system-architecture--technology-stack)
5. [Microkernel IPC Bus & Security Architecture](#5-microkernel-ipc-bus--security-architecture)
6. [Dynamic Theme Engine & Token Specification](#6-dynamic-theme-engine--token-specification)
7. [Declarative Slint UI Implementation](#7-declarative-slint-ui-implementation)
8. [Cross-Platform & Adaptive Device Scaling](#8-cross-platform--adaptive-device-scaling)
9. [Comprehensive Engineering Roadmap](#9-comprehensive-engineering-roadmap)
10. [Monorepo & Repository Directory Layout](#10-monorepo--repository-directory-layout)

---

## 1. Executive Summary & Vision

Commercial smartphone operating systems are designed around variable reward schedules, saturated color palettes, dynamic notification badges, and high-velocity gesture loops. These mechanisms induce compulsive, low-intent device unlocks.

**Mandala Launcher (`mandala-shell`)** is a non-attentional, high-performance mobile shell designed to dismantle these mobile dopamine feedback loops. It replaces conventional icon grids and infinite feeds with a static, text-first, tri-panel spatial canvas.

### Core Philosophy & Performance Targets
* **Anti-Dopamine UX:** Elimination of unread badge counters, dynamic splash screens, and algorithmic feeds.
* **Low-Latency Cold Boots:** Native Rust initialization launching to interactive state in **$< 20\text{ ms}$**.
* **Minimal Memory Footprint:** Total resident set size (RSS) memory footprint maintained at **$< 5\text{ MB}$** on idle.
* **Microkernel Security:** Hardware-isolated module sandboxing through host-mediated Access Control Lists (ACLs) and Ed25519 signature validation.

---

## 2. Cognitive Ergonomics & Behavioral Framework

### 2.1 Physical Touch Reachability (Fitts's Law)
Fitts's Law models target acquisition time as a function of target distance ($D$) and width ($W$):

$$MT = a + b \log_2 \left( \frac{2D}{W} \right)$$

On modern tall mobile displays ($19.5:9$ aspect ratios), single-handed thumb arcs dictate distinct ergonomic zones:

```
+-------------------------------------------------------+ 0% Y-Height
|                  TOP DISPLAY ZONE                     |
|           (Hard Stretch / Two-Handed Reach)           |
|            System Status & Passive Indicators         |
+-------------------------------------------------------+
|                 MID-DISPLAY WORKSPACE                 |
|             (Natural Line-of-Sight Canvas)            |
|                                                       |
|   +-----------------+           +-----------------+   |
|   | Panel B (Left)  |           | Panel C (Right) |   |
|   | Non-Dominant    |           | Dominant        |   |
|   | Stretch Zone    |           | Lateral Sweep   |   |
|   +-----------------+           +-----------------+   |
+-------------------------------------------------------+ 70% Height Threshold
|              NATURAL RESTING THUMB SWEEP              |
|               PANEL A (BOTTOM DOCK)                   |
|             Zero-Friction Intent Zone                 |
+-------------------------------------------------------+ 100% Y-Height
```

### 2.2 Operational Cognitive Modes

| Operational Mode | Trigger Mechanism | Target Utilities | Ergonomic Strategy |
| :--- | :--- | :--- | :--- |
| **Knowing (Intentional)** | Clear, pre-formed real-world task (e.g., take note, run terminal command, compute calculation, retrieve 2FA token). | Terminal, Quick Notes, All Apps Search, 2FA, Vault, Calculator. | **Zero-Friction Access:** Placed in Panel A (Bottom Dock) or Panel C (Primary Lateral Sweep) for instant single-action acquisition. |
| **Unknowing (Impulsive)** | Boredom, emotional escape, muscle-memory checks, sub-conscious seeking loops. | News feeds, RSS streams, weather checks, media gallery. | **Strategic Micro-Friction:** Housed within Panel B (Non-dominant stretch) or behind explicit lower-third threshold swipes to break reflexive habit loops. |

---

## 3. Spatial Architecture & 15-Slot Matrix

The launcher arranges 15 core utilities across 3 panels (5 slots per panel) mapped directly by intention and accessibility.

```
                    +-----------------------------------+
                    |        MAIN WORKSPACE CANVAS      |
                    |     Text Command Query Input      |
                    +-----------------------------------+
                                      │
     ┌────────────────────────────────┼────────────────────────────────┐
     ▼                                ▼                                ▼
+-----------------------+   +-----------------------+   +-----------------------+
|    PANEL B (LEFT)     |   |    PANEL A (BOTTOM)   |   |    PANEL C (RIGHT)    |
|  Passive Context      |   |  High-Intent Anchor   |   |  Utilities & Security |
+-----------------------+   +-----------------------+   +-----------------------+
| B1: Weather           |   | A1: Terminal          |   | C1: 2FA Authenticator |
| B2: RSS Feeder        |   | A2: Quick Notes       |   | C2: Password Vault    |
| B3: Task/Habit Tracker|   | A3: All Apps Drawer   |   | C3: LocalSend (P2P)   |
| B4: Document Reader   |   | A4: Secure Chat       |   | C4: Calculator        |
| B5: Media Gallery     |   | A5: AI Assistant Bar  |   | C5: Camera Capture    |
+-----------------------+   +-----------------------+   +-----------------------+
```

### 15-Slot Module Allocation
* **Panel A (Center Anchor & Bottom Reach Zone):**
  1. `A1: Terminal` – Integrated system shell with customized command aliases.
  2. `A2: Quick Notes` – Raw-text instant input buffer without boot overhead.
  3. `A3: App Drawer` – Keyboard-first fuzzy search launcher replacing grid icons.
  4. `A4: Secure Chat` – Gateway to Signal/Matrix stripped of social feed noise.
  5. `A5: AI Query` – Local on-device LLM query bar for immediate tasks.
* **Panel B (Left Context Drawer):**
  1. `B1: Weather` – Breezy Weather bridge showing minimal environmental metrics.
  2. `B2: RSS Feeder` – Plain-text reader fetching articles without recommendation loops.
  3. `B3: Task Tracker` – Tasks.org integration for todo and habit management.
  4. `B4: Document Reader` – MJ PDF bridge for local offline documentation.
  5. `B5: Media Gallery` – Fossify Gallery bridge for direct local media inspection.
* **Panel C (Right Utility Drawer):**
  1. `C1: 2FA Authenticator` – Aegis Authenticator TOTP token generator.
  2. `C2: Password Vault` – KeePassDX / Strongbox credential manager.
  3. `C3: LocalSend` – Zero-telemetry local peer-to-peer Wi-Fi file sharing.
  4. `C4: Calculator` – Precision scientific calculator and converter.
  5. `C5: Camera Capture` – Open Camera integration saving directly to local disk.

---

## 4. System Architecture & Technology Stack

```
+-------------------------------------------------------------------+
|                    SLINT DECLARATIVE UI LAYER                     |
|   - Native Compiled Graphics (FemtoVG / Vulkan / OpenGL ES)      |
|   - Dynamic Reactive Token Bindings (.slint)                      |
|   - Touch & Gesture Controller                                    |
+-------------------------------------------------------------------+
                                  │ (Native C-ABI / Rust Bindings)
                                  ▼
+-------------------------------------------------------------------+
|                     MANDALA RUST CORE HOST                        |
|   - Unidirectional Data Flow (UDF) State Machine                  |
|   - Microkernel IPC Broker & Access Control Lists (ACL)          |
|   - Ed25519 Dynamic Binary Signature Verifier                     |
|   - Dynamic Theme Engine JSON Parser                              |
+-------------------------------------------------------------------+
                                  │
      ┌───────────────────────────┴───────────────────────────┐
      ▼                                                       ▼
+-----------------------------------+   +-----------------------------------+
|   NATIVE RUST SUB-MODULES         |   |   ANDROID NDK BRIDGE              |
|   (Shared Objects / .so)          |   |   (JNI / DexClassLoader / AOSP)   |
+-----------------------------------+   +-----------------------------------+
```

### Stack Components
* **Core Language:** Pure Rust (`std` + Tokio async runtime).
* **Presentation Engine:** Slint (`slint-ui`) compiled to native machine code via `build.rs`.
* **Dynamic Module Loading:** Monorepo using Git submodules; loaded via `libloading` on Linux or `DexClassLoader` / JNI bridge on Android.
* **Graphics Backends:** Vulkan / OpenGL ES 2.0+ via `FemtoVG` software/hardware fallback.

---

## 5. Microkernel IPC Bus & Security Architecture

Sub-modules execute inside memory sandboxes. Modules cannot interact directly or request system resources without passing through the Host IPC Broker.

```
+---------------------+                           +---------------------+
| SUB-MODULE ALPHA    |                           | SUB-MODULE BETA     |
| (e.g., Quick Notes) |                           | (e.g., Vault C2)    |
+---------------------+                           +---------------------+
           │                                                 ▲
           │ 1. Request Envelope                             │ 4. Deliver Payload
           ▼                                                 │
+-----------------------------------------------------------------------+
|                       HOST REFERENCE MONITOR                          |
|  - Validates Ed25519 Binary Signature                                 |
|  - Evaluates Sender / Target ACL Permission Matrix                    |
+-----------------------------------------------------------------------+
```

### Access Control List (ACL) & Broker Implementation (`ipc_bus.rs`)

```rust
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use serde::{Serialize, Deserialize};
use ed25519_dalek::{VerifyingKey, Signature, Verifier};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct IpcEnvelope {
    pub id: String,
    pub sender: String,
    pub target: String,
    pub action: String,
    pub payload_json: String,
}

pub struct SecurityManager {
    public_key: VerifyingKey,
}

impl SecurityManager {
    pub fn verify_module(&self, binary: &[u8], signature: &[u8]) -> bool {
        if let Ok(sig) = Signature::from_slice(signature) {
            self.public_key.verify(binary, &sig).is_ok()
        } else {
            false
        }
    }
}

#[derive(Clone)]
pub struct AclMatrix {
    pub allowed_routes: HashMap<String, Vec<String>>,
}

pub struct MicrokernelIpcBroker {
    acl: AclMatrix,
    tx: mpsc::Sender<IpcEnvelope>,
    rx: Arc<Mutex<mpsc::Receiver<IpcEnvelope>>>,
}

impl MicrokernelIpcBroker {
    pub fn new(buffer_capacity: usize, acl: AclMatrix) -> Self {
        let (tx, rx) = mpsc::channel(buffer_capacity);
        Self {
            acl,
            tx,
            rx: Arc::new(Mutex::new(rx)),
        }
    }

    pub async fn dispatch(&self, envelope: IpcEnvelope) -> Result<(), &'static str> {
        if let Some(targets) = self.acl.allowed_routes.get(&envelope.sender) {
            if targets.contains(&envelope.target) || envelope.target == "host" {
                self.tx.send(envelope).await.map_err(|_| "IPC Buffer Overrun")?;
                return Ok(());
            }
        }
        Err("ACL Violation: Route unauthorized by Host Reference Monitor")
    }
}
```

---

## 6. Dynamic Theme Engine & Token Specification

Mandala supports live visual skinning via JSON theme manifests without binary recompilation or runtime restarts.

### 6.1 Theme Manifest JSON Schema (`theme.json`)

```json
{
  "theme_metadata": {
    "id": "org.mandala.theme.retro_terminal",
    "name": "Retro Terminal",
    "version": "1.0.0"
  },
  "color_tokens": {
    "canvas_bg": "#0D1117",
    "surface_primary": "#161B22",
    "surface_border": "#30363D",
    "text_primary": "#00FF66",
    "text_secondary": "#008833",
    "accent_focus": "#00FF66"
  },
  "style_tokens": {
    "border_width_px": 1.0,
    "corner_radius_px": 2.0,
    "font_family": "SpaceMono-Regular"
  }
}
```

### 6.2 Supported Preset Styles
1. **Minimalist OLED:** Dark background (`#000000`), low-contrast text (`#E0E0E0`), amber accent (`#D4A373`).
2. **Cartoonish / Cel-Shaded:** Warm background (`#FFF9EC`), thick border outlines (`3px #111111`), pastel highlight (`#FF6B6B`).
3. **Retro Terminal:** Monospaced phosphorescent green (`#00FF66`) on dark slate (`#0D1117`).

---

## 7. Declarative Slint UI Implementation

### 7.1 Global Theme Schema (`ui/theme_tokens.slint`)

```slint
export global ThemePalette {
    in-out property <color> canvas-bg: #121417;
    in-out property <color> surface-primary: #1A1D23;
    in-out property <color> surface-border: #2A2E37;
    in-out property <color> text-primary: #D0D4DC;
    in-out property <color> text-secondary: #7A828E;
    in-out property <color> accent-focus: #94A8B8;
    in-out property <length> border-width: 0px;
    in-out property <length> corner-radius: 4px;
    in-out property <string> font-family: "Inter";
}
```

### 7.2 Main Window & Panel Gestures (`ui/main_window.slint`)

```slint
import { ThemePalette } from "theme_tokens.slint";

export component MainWindow inherits Window {
    width: 412px;
    height: 915px;
    background: ThemePalette.canvas-bg;

    in-out property <bool> left-panel-open: false;
    in-out property <bool> right-panel-open: false;

    callback dispatch-action(string, string);

    // Panel A - Main Intent Workspace
    Rectangle {
        width: 100%;
        height: 100%;

        VerticalLayout {
            padding: 24px;
            spacing: 16px;

            // Search Bar Input
            Rectangle {
                height: 48px;
                background: ThemePalette.surface-primary;
                border-radius: ThemePalette.corner-radius;
                border-width: ThemePalette.border-width;
                border-color: ThemePalette.surface-border;

                TextInput {
                    text: "Search or type command...";
                    color: ThemePalette.text-secondary;
                    font-size: 14px;
                    vertical-alignment: center;
                    x: 16px;
                }
            }

            Rectangle { vertical-stretch: 1; }

            // Panel A Bottom Dock
            HorizontalLayout {
                height: 64px;
                spacing: 8px;

                Rectangle {
                    background: ThemePalette.surface-primary;
                    border-radius: ThemePalette.corner-radius;
                    
                    TouchArea {
                        clicked => { root.dispatch-action("app-terminal", "launch"); }
                    }
                    Text {
                        text: "TERM";
                        color: ThemePalette.text-primary;
                        horizontal-alignment: center;
                        vertical-alignment: center;
                    }
                }
            }
        }
    }

    // Panel B Drawer (Left Context)
    Rectangle {
        width: 280px;
        height: 100%;
        x: root.left-panel-open ? 0px : -280px;
        animate x { duration: 180ms; easing: ease-out; }
        background: ThemePalette.surface-primary;
        border-width: ThemePalette.border-width;
        border-color: ThemePalette.surface-border;
    }

    // Panel C Drawer (Right Utilities)
    Rectangle {
        width: 280px;
        height: 100%;
        x: root.right-panel-open ? root.width - self.width : root.width;
        animate x { duration: 180ms; easing: ease-out; }
        background: ThemePalette.surface-primary;
        border-width: ThemePalette.border-width;
        border-color: ThemePalette.surface-border;
    }
}
```

---

## 8. Cross-Platform & Adaptive Device Scaling

### 8.1 Multi-Display Breakpoint System
```slint
export component ScreenScaleEngine {
    in property <length> window-width;

    out property <bool> is-compact: root.window-width < 600px;
    out property <bool> is-medium: root.window-width >= 600px && root.window-width < 840px;
    out property <bool> is-expanded: root.window-width >= 840px;
}
```

* **Compact Screens (Standard Phones):** Panels B and C operate as overlay drawers triggered by lower-30% edge swipes.
* **Medium Screens (Foldables Unfolded):** Panel A remains centered, while Panel B or Panel C can dock persistently alongside the workspace.
* **Expanded Screens (Tablets / Desktop Linux):** Tiled layout displaying Panel B, Panel A, and Panel C simultaneously across three columns.

---

## 9. Comprehensive Engineering Roadmap

```
2026 Q4       2027 Q1       2027 Q2       2027 Q3       2027 Q4
  │             │             │             │             │
  ├───Phase 1───┤             │             │             │
  │ Shell & UI  │             │             │             │
  │             ├───Phase 2───┤             │             │
  │             │ IPC & Bus   │             │             │
  │             │             ├───Phase 3───┤             │
  │             │             │ Submodules  │             │
  │             │             │             ├───Phase 4───┤
  │             │             │             │ Themes & UI │
  │             │             │             │             ├───Phase 5───►
  │             │             │             │             │ NDK & Release
```

### Phase 1: Core Shell & Slint Tri-Panel Layout
- [x] Configure Cargo monorepo workspace and build scripts (`slint-build`).
- [x] Implement spatial layout with Panel A, Panel B, and Panel C.
- [x] Establish gesture height isolation filters (upper 70% vs. lower 30%).

### Phase 2: Microkernel IPC Bus & Security Engine
- [x] Implement Tokio async channel message router. (`core/router.rs`, 2 tests; broker/ACL tests in `core/ipc_bus.rs`)
- [x] Integrate Access Control List (ACL) permission verification. (`core/ipc_bus.rs`)
- [x] Add Ed25519 signature checks for external dynamic modules. (`core/crypto.rs`, `core/loader.rs`)

### Phase 3: Dynamic Module Integration
- [x] Connect `A1: Terminal` embedded shell. (`modules/app-terminal`, no process spawn: sandboxed command set)
- [x] Connect `A2: Quick Notes` raw-text buffer. (`modules/app-notes`)
- [x] Implement `A3: App Drawer` fuzzy finder. (host builtin: `core/command.rs` + `state.rs`)
- [x] Integrate `C1: Aegis` 2FA, `C2: KeePassDX`, `C3: LocalSend`.
  - `modules/app-aegis`: RFC 6238 TOTP, imports Aegis/Google `otpauth://` URIs.
  - `modules/app-vault`: Argon2id + XChaCha20-Poly1305 store, imports the KeePassDX CSV export (KDBX is not parsed).
  - `modules/app-localsend`: token-guarded one-shot TCP transfer with SHA-256 verification (not the LocalSend wire protocol).

### Phase 4: Theme Engine & Dynamic Token Injection
- [x] Implement Serde JSON theme parser in Rust. (`core/theme.rs`)
- [x] Connect real-time property mutation to Slint global `ThemePalette`. (`core/theme.rs` -> `ui/src/lib.rs`)
- [x] Validate theme presets (OLED Dark, Cel-Shaded, Retro Terminal). (`themes/*.json`, 5 tests)

### Phase 5: Android NDK Packaging & Optimization
- [x] Configure `cargo-ndk` build pipelines for `aarch64-linux-android`. (`scripts/android-build.sh`, `.github/workflows/release.yml`)
- [x] Implement Android system overlay and wallpaper permissions via JNI. (`shell/src/android.rs`; needs a device smoke test)
- [x] Conduct performance profiling of cold boot and RAM footprint. (`tools/mandala-perf`, shell boot log; see `docs/performance.md` — the 20 ms / 5 MB targets are design goals the GUI stack does not meet)

---

## 9.1 Implementation status (evidence)

The checklist above is backed by `cargo test --workspace` in this repository.
Deliberate deviations from the text of this document:

| Design text | Shipped behaviour | Why |
|---|---|---|
| `DexClassLoader` / `.dex` modules (Section 4) | Native `cdylib` modules loaded with `libloading` on every platform | One ABI, one loader, one signature pipeline; the in-process trust model is identical |
| Remote fetch from Git branches (Section 6) | Local signed files in `<home>/modules` | A downloader can drop files there; verification already handles the rest |
| KeePassDX KDBX database | KeePassDX **CSV** import into the encrypted vault | KDBX needs a large format/crypto stack; CSV is the app's export path |
| LocalSend wire protocol | Token-guarded direct TCP transfer with SHA-256 | Keeps the LAN-only / no-telemetry property without a multicast discovery stack |
| 15 slot modules | 6 modules (`notes`, `calculator`, `terminal`, `aegis`, `vault`, `localsend`) + builtin search | Slots without a signed module report "not installed" by design |

---

## 10. Monorepo & Repository Directory Layout

```
mandala-shell/
├── .gitmodules                 # Submodule pointer definitions
├── Cargo.toml                  # Cargo Workspace Root Configuration
├── docs/
│   └── ARCHITECTURE_AND_ROADMAP.md
├── mandala-core/               # Rust Backend Host Engine
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs             # Application Bootstrap & Event Loop
│       ├── ipc_bus.rs          # Access Control List Message Router
│       ├── security.rs         # Ed25519 Binary Verifier
│       └── theme_loader.rs     # JSON Theme Manifest Parser
├── mandala-ui/                 # Slint GUI Rendering Package
│   ├── build.rs                # Compiles .slint markup to Rust
│   ├── Cargo.toml
│   └── ui/
│       ├── main_window.slint   # Tri-Panel Layout Manager
│       ├── theme_tokens.slint  # Global Theme Property Bindings
│       └── panels/
│           ├── panel_a.slint   # Center Anchor Component
│           ├── panel_b.slint   # Left Drawer Component
│           └── panel_c.slint   # Right Drawer Component
└── submodules/                 # Dynamic Modules (Git Submodules)
    ├── app-terminal/
    ├── app-notes/
    ├── app-drawer/
    └── app-aegis/
```
