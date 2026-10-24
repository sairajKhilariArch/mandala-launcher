# Mandala Launcher (`mandala-shell`): Comprehensive Behavioral, Ergonomic, and Architectural Research Guide

**Core Stack:** Pure Rust Core Engine | Slint Declarative Native UI Framework | Dynamic Sub-Module Architecture

---

## Executive Summary

Modern mobile operating systems are engineered around engagement optimization, using high-saturation visual cues, unpredictable reward mechanics, and friction-free application launching to induce reflexive device usage. **Mandala Launcher (`mandala-shell`)** is a non-attentional, privacy-first, tri-panel mobile shell designed to dismantle these mobile dopamine loops.

Built on **Rust** and **Slint**, this architecture introduces:

1. **Fitts's Law-Compliant Ergonomics:** Spatial partitioning based on physical thumb zones and intentional versus impulsive cognitive states.
2. **Hypervisor-Style Host IPC:** Microkernel-level sandboxing between decoupled application modules via strict Access Control Lists (ACLs).
3. **Dynamic Module Lifecycle:** A Git-backed modular plugin architecture pulling pre-compiled binaries on demand without host re-compilation.
4. **Adaptive Multi-Display Engine:** Dynamic layout scaling across single-screen phones, foldables, tri-folds, and large-format displays.
5. **Declarative Theme Engine:** Reactive visual skinning supporting Minimalist OLED, Cel-Shaded Cartoonish, Retro Terminal, and User-Defined JSON Themes.

---

## Section 1: Behavioral Ergonomics & Cognitive UX Framework

### 1.1 Touch Ergonomics & Fitts's Law Analysis

Fitts's Law states that the target acquisition time is a function of target distance and target size:

$$MT = a + b \log_2 \left( \frac{2D}{W} \right)$$

On mobile displays, physical hand movement constraints divide the screen into distinct reachability zones.

```
+-------------------------------------------------------+
|                    TOP DISPLAY ZONE                   |
|               (Hard Stretch / Two-Hand)               |
|            System Notifications & Status              |
+-------------------------------------------------------+
|                 MID-DISPLAY WORKSPACE                 |
|             (Natural Line of Sight / Canvas)          |
|                                                       |
|   +-----------------+           +-----------------+   |
|   | Panel B (Left)  |           | Panel C (Right) |   |
|   | Non-Dominant    |           | Dominant        |   |
|   | Sweep/Stretch   |           | Sweep           |   |
|   +-----------------+           +-----------------+   |
+-------------------------------------------------------+
|                 PANEL A (BOTTOM DOCK)                 |
|            PRIMARY RESTING THUMB SWEEP ZONE           |
|            Instant Intentional Access                 |
+-------------------------------------------------------+
```

* **Natural Reach Zone (Bottom 30% - Panel A):** Thumb rests here naturally. Zero hand re-gripping required. Ideal for zero-friction intentional utilities.
* **Dominant Lateral Sweep (Side Border - Panel C / Panel B):** Reached via lateral thumb arcs. Best for quick secondary utility lookup.
* **Non-Dominant Stretch Zone (Opposite Side Border):** Requires explicit hand shift or two-handed interaction. Used strategically to introduce micro-friction against impulsive habits.

### 1.2 "Knowing" vs. "Unknowing" Usage Mechanics

| Cognitive Mode | Trigger Mechanism | Target Applications | UX Strategy & Placement |
| :--- | :--- | :--- | :--- |
| **Knowing / Intentional** | Real-world goal (e.g., capture note, execute command, copy 2FA code). | Terminal, Notes, 2FA (Aegis), Calculator, Password Vault. | **Zero Friction:** Fixed in Panel A or Panel C for instant access. |
| **Unknowing / Impulsive** | Dopamine seeking, boredom, muscle-memory loops, visual notifications. | News feeds, RSS, social media, gallery checking. | **Micro-Friction:** Placed in Panel B (non-dominant stretch) or behind explicit lower-third edge gestures. |

---

## Section 2: Complete 15-Slot Tri-Panel Application Matrix

The interface maps 15 discrete core applications across 3 panels (5 slots per panel) according to cognitive intent and reachability:

```
                  +-----------------------------------+
                  |        MAIN WORKSPACE CANVAS      |
                  |     Command Query / Input Field   |
                  +-----------------------------------+
                                    |
   +--------------------------------+--------------------------------+
   |                                |                                |
   v                                v                                v
+-----------------------+ +-----------------------+ +-----------------------+
|    PANEL B (LEFT)     | |    PANEL A (BOTTOM)   | |    PANEL C (RIGHT)    |
| Passive Information   | | High-Intent Core     | | Active Utilities & Sec|
+-----------------------+ +-----------------------+ +-----------------------+
| B1: Weather           | | A1: Terminal          | | C1: 2FA Authenticator |
| B2: RSS Feeder        | | A2: Quick Notes     | | C2: Password Vault  |
| B3: Task/Habit Tracker| | A3: All Apps Drawer | | C3: LocalSend (File)|
| B4: PDF / Doc Reader  | | A4: Secure Chat     | | C4: Calculator      |
| B5: Media Gallery     | | A5: AI Assistant    | | C5: Camera Capture  |
+-----------------------+ +-----------------------+ +-----------------------+
```

### Detailed Panel Allocation

#### Panel A: Bottom Dock (High-Intent Core)
1. **Terminal:** Embedded command line for shell automation, custom scripts, and direct system control.
2. **Notes / Micro-Capture:** Instant raw-text buffer bypassing app-boot overhead for rapid thought capture.
3. **All Apps Drawer:** Fuzzy-search keyboard launcher avoiding visual grid icons.
4. **Secure Chat:** Minimalist messaging gateway stripped of stories, status updates, and social feeds.
5. **AI Assistant:** On-device contextual language processor and task automation pipeline.

#### Panel B: Left Panel (Passive Information & Retrieval)
1. **Weather:** Passive, non-tracked environmental awareness.
2. **RSS / News Feeder:** Reader-mode news fetching without algorithmic recommendation loops.
3. **Task & Habit Tracker:** Plain-text or structured todo management.
4. **Document / PDF Reader:** Local reader for offline documentation.
5. **Media Gallery:** Minimalist local image/video file viewer.

#### Panel C: Right Panel (Active Utilities & Security)
1. **2FA Authenticator (Aegis):** Time-based one-time password (TOTP) generator.
2. **Password Vault (KeePassDX/Strongbox):** Encrypted local credential store.
3. **LocalSend File Share:** Zero-telemetry local peer-to-peer Wi-Fi transfer.
4. **Calculator & Unit Converter:** High-precision offline scientific utility.
5. **Camera Capture:** Hardware camera capture saving directly to local storage without cloud sync.

---

## Section 3: Universal Gesture Taxonomy & Navigation Mapping

To eliminate conflicts with native OS navigation (Android/iOS Edge Back gestures), gestures are spatially partitioned using screen height thresholds:

```
+-------------------------------------------------------+ 0% Top Display
|                                                       |
|                                                       |
|             UPPER 70% OF DISPLAY BORDERS              |
|        Reserved for System Navigation Gestures        |
|               (Native OS "Back" Swipe)                |
|                                                       |
|                                                       |
+-------------------------------------------------------+ 70% Height Threshold
|              LOWER 30% EDGE SWIPE ZONES               |
| [Swipe Inward Left]                       [Swipe Inward Right]
|   Expands Panel B                           Expands Panel C
+-------------------------------------------------------+ 100% Bottom Anchor
|               [Single Swipe Up]                       |
|          Minimize App / Return to Home                |
+-------------------------------------------------------+
```

### Gesture Engine Mapping
1. **Lower 30% Inward Edge Swipe (Left/Right):** Opens Panel B or Panel C respectively.
2. **Upper 70% Inward Edge Swipe (Left/Right):** Passed through directly to OS window manager for native "Back" navigation.
3. **Single Upward Drag from Bottom Anchor:** Minimizes the active sub-module and clears focus to the main workspace.
4. **Double Upward Drag / Hold:** Opens the Module Process Monitor to freeze, restart, or unload background sub-modules.

---

## Section 4: Slint UI Framework Integration & Code Architecture

### 4.1 Declarative Theme Schema (`ui/theme_tokens.slint`)

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

### 4.2 Responsive Tri-Panel Main Layout (`ui/main_window.slint`)

```slint
import { ThemePalette } from "theme_tokens.slint";

export component MainWindow inherits Window {
    width: 412px;
    height: 915px;
    background: ThemePalette.canvas-bg;

    // Panel Drawer Visibilities driven by Rust State Machine
    in-out property <bool> left-panel-open: false;
    in-out property <bool> right-panel-open: false;

    callback trigger-ipc-event(string, string); // (Target Module, Action Payload)

    // Center Main Workspace (Panel A + Command Canvas)
    Rectangle {
        width: 100%;
        height: 100%;
        background: ThemePalette.canvas-bg;

        VerticalLayout {
            padding: 24px;
            spacing: 16px;

            // Command input / Search bar
            TextInput {
                height: 48px;
                color: ThemePalette.text-primary;
                font-family: ThemePalette.font-family;
                vertical-alignment: center;
            }

            // Central Canvas Area
            Rectangle {
                vertical-stretch: 1;
            }

            // Panel A Bottom Dock Anchor (5 Core Buttons)
            HorizontalLayout {
                height: 64px;
                spacing: 8px;

                Rectangle {
                    background: ThemePalette.surface-primary;
                    border-radius: ThemePalette.corner-radius;
                    border-width: ThemePalette.border-width;
                    border-color: ThemePalette.surface-border;
                    
                    TouchArea {
                        clicked => { root.trigger-ipc-event("app-terminal", "launch"); }
                    }
                    Text {
                        text: "TERM";
                        color: ThemePalette.text-primary;
                        vertical-alignment: center;
                        horizontal-alignment: center;
                    }
                }
            }
        }
    }

    // Left Drawer (Panel B) - Animated Edge Panel
    Rectangle {
        width: 280px;
        height: 100%;
        x: root.left-panel-open ? 0px : -280px;
        animate x { duration: 180ms; easing: ease-out; }
        background: ThemePalette.surface-primary;
        border-color: ThemePalette.surface-border;
        border-width: ThemePalette.border-width;
    }

    // Right Drawer (Panel C) - Animated Edge Panel
    Rectangle {
        width: 280px;
        height: 100%;
        x: root.right-panel-open ? root.width - self.width : root.width;
        animate x { duration: 180ms; easing: ease-out; }
        background: ThemePalette.surface-primary;
        border-color: ThemePalette.surface-border;
        border-width: ThemePalette.border-width;
    }
}
```

---

## Section 5: Rust Core Engine & Microkernel IPC Bus

### 5.1 System Layering Structure

```
+---------------------------------------------------------------+
|                      SLINT NATIVE UI LAYER                    |
|    Renders canvas, captures gestures, exposes properties      |
+---------------------------------------------------------------+
                                | (Rust Function Callbacks)
                                v
+---------------------------------------------------------------+
|                       RUST HOST ENGINE                        |
|  +---------------------------------------------------------+  |
|  | Unidirectional Data Flow (UDF) State Machine            |  |
|  +---------------------------------------------------------+  |
|  | Microkernel IPC Message Broker (Tokio Channels + ACL)  |  |
|  +---------------------------------------------------------+  |
|  | Ed25519 Dynamic Binary Loader & Verifier                |  |
|  +---------------------------------------------------------+  |
|  | Theme Engine Serde JSON Parser & Tokens Injector        |  |
+---------------------------------------------------------------+
```

### 5.2 Microkernel IPC Broker Code (`mandala-core/src/ipc_bus.rs`)

```rust
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct IpcEnvelope {
    pub message_id: String,
    pub sender_module: String,
    pub target_module: String,
    pub payload_action: String,
    pub data_json: String,
}

#[derive(Clone)]
pub struct AccessControlList {
    pub allowed_routes: HashMap<String, Vec<String>>,
}

pub struct MicrokernelIpcBroker {
    acl: AccessControlList,
    tx: mpsc::Sender<IpcEnvelope>,
    rx: Arc<Mutex<mpsc::Receiver<IpcEnvelope>>>,
}

impl MicrokernelIpcBroker {
    pub fn new(buffer_size: usize, acl: AccessControlList) -> Self {
        let (tx, rx) = mpsc::channel(buffer_size);
        Self {
            acl,
            tx,
            rx: Arc::new(Mutex::new(rx)),
        }
    }

    pub async fn dispatch(&self, envelope: IpcEnvelope) -> Result<(), &'static str> {
        if let Some(allowed_targets) = self.acl.allowed_routes.get(&envelope.sender_module) {
            if allowed_targets.contains(&envelope.target_module) || envelope.target_module == "host" {
                self.tx.send(envelope).await.map_err(|_| "IPC channel send failure")?;
                return Ok(());
            }
        }
        Err("ACL Violation: Module unauthorized to communicate along this route")
    }
}
```

---

## Section 6: Modular Branch-per-App & Dynamic Loading Architecture

### 6.1 Git Monorepo with Submodules

```
Main Repository (main branch - Bare Minimum Host)
 ├── mandala-core/
 ├── mandala-ui/
 └── submodules/
      ├── app-terminal  [Submodule branch: module/terminal]
      ├── app-notes     [Submodule branch: module/notes]
      ├── app-aegis     [Submodule branch: module/aegis]
      └── app-localsend [Submodule branch: module/localsend]
```

### 6.2 Binary Verification & Loading Execution Flow

```
User Taps Slot (e.g., Panel C Slot 1: Aegis 2FA)
                       │
                       v
         Is Module Cached Locally?
               /               \
             NO                 YES
             /                   \
            v                     v
Fetch Remote Release     Read Local `.so`/`.dex` Module
Artifacts from Branch            │
            │                     │
            +----------<----------+
                       │
                       v
    Compute Binary SHA-256 Digest Hash
                       │
                       v
 Verify Cryptographic Signature against Host Public Key (Ed25519)
                       │
         Is Signature Valid?
               /               \
             YES                NO
             /                   \
            v                     v
Load Module via Dynamic       Quarantine File & Throw
Classloader / libloading      Security Warning in UI
            │
            v
Mount Module into Host Canvas
```

---

## Section 7: Multi-Display Adaptive Responsiveness

```slint
export component ResponsiveLayoutEngine {
    in property <length> window-width;

    out property <bool> is-compact: root.window-width < 600px;
    out property <bool> is-medium: root.window-width >= 600px && root.window-width < 840px;
    out property <bool> is-expanded: root.window-width >= 840px;
}
```

---

## Section 8: Dynamic Theme Engine & Customization Specification

### 8.1 Theme Manifest JSON Schema (`theme.json`)

```json
{
  "theme_id": "minimalist-oled-dark",
  "theme_name": "Minimalist OLED",
  "version": "1.0.0",
  "author": "Mandala Team",
  "tokens": {
    "canvas_bg": "#000000",
    "surface_primary": "#121212",
    "surface_border": "#242424",
    "text_primary": "#E0E0E0",
    "text_secondary": "#7D7D7D",
    "accent_focus": "#D4A373",
    "border_width_px": 0,
    "corner_radius_px": 4,
    "font_family": "Inter Display"
  }
}
```

### 8.2 Preset Styles
1. **Minimalist OLED (Default):** `#000000` background, `#E0E0E0` soft text, `#D4A373` amber accent.
2. **Cartoonish / Cel-Shaded:** `#FFF9EC` cream fill, `#111111` bold borders, `#FF6B6B` pastel accents.
3. **Retro Terminal:** `#0D1117` dark slate, `#00FF66` phosphor green text.
4. **User-Customized:** Dynamic JSON loading via Slint properties at runtime.

---

## Section 9: Implementation Roadmap & Execution Phases

```
PHASE 1: Core Architecture & Slint Layout Framework
 ├── Setup Cargo workspace & Slint compilation step (build.rs)
 ├── Construct MainWindow.slint layout (Panels A, B, C & Gesture Thresholds)
 └── Implement responsive width breakpoint bindings

PHASE 2: Microkernel IPC Bus & State Machine
 ├── Build Tokio async channel event bus with Access Control Lists (ACL)
 ├── Integrate Slint event handling callbacks into Rust main thread
 └── Implement safe-state handling and uncaught exception isolation

PHASE 3: Modular Sub-App Engine & Security Pipeline
 ├── Setup Git Monorepo with Submodule branch links
 ├── Implement Ed25519 signature verification step in Rust core
 └── Build dynamic module loader and offline permission enforcer

PHASE 4: Dynamic Theme Engine & Polish
 ├── Build Serde JSON theme manifest parser
 ├── Implement reactive Slint theme property injector
 └── Conduct Fitts's Law thumb ergonomics validation across device sizes
```
