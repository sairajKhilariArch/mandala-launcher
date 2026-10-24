# ☸️ Mandala Launcher (`mandala-shell`)

[![Build Status](https://img.shields.io/badge/build-passing-brightgreen)](#)
[![Language](https://img.shields.io/badge/language-Rust-orange)](https://www.rust-lang.org/)
[![UI Framework](https://img.shields.io/badge/UI-Slint-blue)](https://slint.dev/)
[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue)](#)

> **Mandala Launcher** is a non-attentional, high-performance, modular mobile launcher framework built on a pure Rust core and rendered via Slint UI. Designed to eliminate mobile distraction through a static, text-first tri-panel spatial architecture.

---

### 🌟 Key Features

* 📱 **Tri-Panel Ergonomics:** Spatial layout splitting actions into Center Intent (Panel A), Left Context/Feeds (Panel B), and Right Utilities (Panel C).
* ⚡ **Rust + Slint Architecture:** Native compilation with < 5MB memory footprint and instant cold boots.
* 🛡️ **Microkernel IPC Bus:** Hypervisor-style Access Control Lists (ACL) preventing unauthorized cross-module memory access.
* 🎨 **Dynamic Theme Engine:** Live-skinning JSON theme system supporting Minimalist OLED, Cel-Shaded Cartoonish, and Retro Terminal styles.
* 📦 **Git Branch Sub-Modules:** Isolated dynamic feature modules fetched and loaded securely via Ed25519 signature checks.
