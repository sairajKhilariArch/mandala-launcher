<div align="center">

# ☸️ Mandala Launcher (`mandala-shell`)

**A Non-Attentional, Microkernel-Secured, Modular Tri-Panel Mobile Shell**

*Engineered to dismantle mobile dopamine loops, prioritize intentional computing, and deliver high-performance user experience.*

[![Language: Rust](https://img.shields.io/badge/Core-Rust_1.75+-orange?style=flat-square&logo=rust)](https://www.rust-lang.org/)
[![UI Framework: Slint](https://img.shields.io/badge/UI-Slint_1.4+-blue?style=flat-square)](https://slint.dev/)
[![Architecture: Microkernel IPC](https://img.shields.io/badge/Architecture-Microkernel_IPC-purple?style=flat-square)](#-microkernel-ipc-bus--security)
[![License: MIT/Apache-2.0](https://img.shields.io/badge/License-MIT%2FApache--2.0-green?style=flat-square)](#-license)

[Features](#-key-features) •
[Ergonomic Framework](#-ergonomic--behavioral-ux-framework) •
[Tech Stack](#-technology-stack) •
[Architecture](#-system-architecture) •
[Getting Started](#-getting-started) •
[Theme System](#-dynamic-theme-engine)

</div>

---

## 📖 Overview

Modern smartphone interfaces are engineered around engagement optimization—leveraging saturated icons, notification badges, unpredictable feeds, and high-frequency animations to capture user focus.

**Mandala Launcher (`mandala-shell`)** is a radical departure from commercial mobile launcher design. Built on a pure **Rust** backend and rendered with the **Slint UI** framework, Mandala transforms your home screen into a static, text-first, intent-driven canvas. 

By applying **Fitts’s Law** to spatial ergonomics and decoupling application execution into a **hypervisor-style microkernel architecture**, Mandala provides a quiet, fast and secure mobile workspace.

---

## 🌟 Key Features

* 📐 **Tri-Panel Spatial Canvas:** Eliminates deep app drawer stacks by dividing the screen into three persistent zones:
  * **Panel A (Center Anchor):** Text-first command terminal, search bar, and primary intent utilities.
  * **Panel B (Left Context Drawer):** Passive information retrieval (RSS, Weather, Tasks).
  * **Panel C (Right Utility Drawer):** Security tools, local syncing, and diagnostics (2FA, KeePass, LocalSend).
* ⚡ **Rust + Slint Native Performance:** Zero garbage-collection stutters and native startup. Measured cold-boot and memory numbers — including the gap to the original 20 ms / 5 MB targets — are in [`docs/performance.md`](docs/performance.md).
* 🛡️ **Microkernel IPC Bus:** Hypervisor-style Access Control Lists (ACL) enforce strict memory sandboxing between dynamic modules.
* 🎨 **Dynamic Theme Engine:** Hot-swappable JSON design manifests supporting **Minimalist OLED**, **Cartoonish Cel-Shaded**, and **Retro Terminal** themes.
* 🌿 **Monorepo & Submodule Integration:** Modular "branch-per-app" architecture powered by Git submodules for isolated feature testing and building.

---

## 🖐 Ergonomic & Behavioral UX Framework

### Cognitive Modes: "Knowing" vs. "Unknowing" Actions

Mandala partitions phone usage into two operational modes to eliminate mindless swiping:

1. **Intentional (Knowing) Usage:** High-priority, purposeful actions (e.g., launching terminal, searching an app, or taking a quick note) are placed in the **Natural Thumb Reach Zone (Bottom 30%)**.
2. **Impulsive (Unknowing) Usage:** Passive checking behavior is suppressed by placing information feeds behind deliberate lateral swipe gestures in the **Non-Dominant Stretch Zones**.
