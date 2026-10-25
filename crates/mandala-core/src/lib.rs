//! `mandala-core`: UI-agnostic host engine of the Mandala launcher.
//!
//! * [`ipc_bus`]  – ACL-enforcing message broker
//! * [`router`]   – async consumer that dispatches to modules
//! * [`loader`]   – Ed25519-verified dynamic module loading + process monitor
//! * [`crypto`]   – SHA-256 / Ed25519 helpers and trust store
//! * [`state`]    – pure UDF reducer (`Action -> Effect`)
//! * [`gesture`]  – thumb-zone gesture classifier
//! * [`command`]  – fuzzy search and command grammar
//! * [`slots`]    – the 15-slot tri-panel matrix
//! * [`theme`]    – JSON theme manifests and presets
//! * [`paths`]    – on-disk layout
//! * [`sys`]      – process RSS introspection (performance budget)
//! * [`catalog`]  – module store registry format and validation

pub mod catalog;
pub mod command;
pub mod crypto;
pub mod error;
pub mod gesture;
pub mod ipc_bus;
pub mod loader;
pub mod paths;
pub mod router;
pub mod slots;
pub mod state;
pub mod sys;
pub mod theme;

pub use error::{MandalaError, Result};
