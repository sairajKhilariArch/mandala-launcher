//! Module store catalog: the registry JSON served over HTTPS (GitHub Pages)
//! and consumed by the in-app Store.
//!
//! The catalog is *discovery*, not authority: a binary still has to carry a
//! signature that verifies against the launcher's baked-in trust key, at
//! install time and again at load time. This module validates structure so a
//! truncated, tampered or path-travelling registry fails fast with a message
//! the Store can show verbatim.

use crate::error::{MandalaError, Result};
use crate::loader::validate_module_id;
use std::collections::HashSet;

/// Bump only for breaking layout changes; the launcher rejects other values.
pub const SCHEMA: u32 = 1;
/// Per-file cap. The largest module today is ~2.4 MB; 64 MB is generous.
pub const MAX_MODULE_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CatalogEntry {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub description: String,
    pub version: String,
    pub sha256: String,
    pub size: u64,
    /// Site-relative path of the binary (joined onto the base URL by the
    /// installer); never an absolute or scheme'd URL.
    pub url: String,
    /// Site-relative path of the detached Ed25519 signature (hex or raw).
    pub sig: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Catalog {
    pub schema: u32,
    pub abi: String,
    #[serde(default)]
    pub generated: String,
    pub modules: Vec<CatalogEntry>,
}

impl Catalog {
    /// Parse and fully validate `json` as the catalog for `abi`.
    pub fn parse(json: &str, abi: &str) -> Result<Self> {
        let cat: Catalog = serde_json::from_str(json)
            .map_err(|e| MandalaError::Store(format!("catalog JSON: {e}")))?;
        cat.validate(abi)?;
        Ok(cat)
    }

    /// Validate every field the installer will later depend on. Returns the
    /// first violation — the Store shows it as-is, so the wording matters.
    pub fn validate(&self, abi: &str) -> Result<()> {
        let store = |m: String| MandalaError::Store(m);
        if self.schema != SCHEMA {
            return Err(store(format!(
                "catalog schema {} (launcher expects {SCHEMA})",
                self.schema
            )));
        }
        if self.abi != abi {
            return Err(store(format!(
                "catalog is for `{}` but `{abi}` was requested",
                self.abi
            )));
        }
        if self.modules.len() > 512 {
            return Err(store(format!(
                "catalog lists {} modules (max 512)",
                self.modules.len()
            )));
        }
        let mut seen = HashSet::new();
        for e in &self.modules {
            validate_module_id(&e.id)
                .map_err(|_| store(format!("entry with invalid id `{}`", e.id)))?;
            if !seen.insert(e.id.as_str()) {
                return Err(store(format!("duplicate id `{}`", e.id)));
            }
            if e.label.trim().is_empty() || e.label.len() > 80 {
                return Err(store(format!("`{}`: label must be 1..=80 bytes", e.id)));
            }
            if e.description.len() > 400 {
                return Err(store(format!(
                    "`{}`: description longer than 400 bytes",
                    e.id
                )));
            }
            if e.version.trim().is_empty() || e.version.len() > 32 {
                return Err(store(format!("`{}`: version must be 1..=32 bytes", e.id)));
            }
            if e.sha256.len() != 64 || hex::decode(&e.sha256).is_err() {
                return Err(store(format!(
                    "`{}`: sha256 must be 64 hex characters",
                    e.id
                )));
            }
            if e.size == 0 || e.size > MAX_MODULE_BYTES {
                return Err(store(format!(
                    "`{}`: size {} outside 1..={MAX_MODULE_BYTES}",
                    e.id, e.size
                )));
            }
            for (field, v) in [("url", &e.url), ("sig", &e.sig)] {
                if !is_safe_relpath(v) {
                    return Err(store(format!(
                        "`{}`: {field} must be a relative path inside the site: `{v}`",
                        e.id
                    )));
                }
            }
        }
        Ok(())
    }

    pub fn entry(&self, id: &str) -> Option<&CatalogEntry> {
        self.modules.iter().find(|e| e.id == id)
    }
}

/// Relative, site-local path: no scheme, no absolute root, no `..`, no
/// backslashes — an entry can only point at a file the publisher placed next
/// to the registry, joined onto the configured base URL.
fn is_safe_relpath(v: &str) -> bool {
    !v.is_empty()
        && v.len() <= 256
        && !v.contains("://")
        && !v.contains('\\')
        && !v.starts_with('/')
        && !v.split('/').any(|p| p == "..")
}

/// Registry ABI name for the running platform. `None` on desktop, where the
/// Store is browse-only and `scripts/dev-setup.sh` owns module installs.
pub fn current_abi() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("android", "aarch64") => Some("android-arm64"),
        ("android", "x86_64") => Some("android-x86_64"),
        _ => None,
    }
}

/// ELF `e_machine` (little-endian u16 at offset 18) that a binary of this
/// ABI must declare. The installer checks it before trusting a download.
pub fn abi_e_machine(abi: &str) -> Option<u16> {
    match abi {
        "android-arm64" => Some(183), // EM_AARCH64
        "android-x86_64" => Some(62), // EM_X86_64
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry() -> CatalogEntry {
        CatalogEntry {
            id: "app-notes".into(),
            label: "Notes".into(),
            description: "Local notes.".into(),
            version: "1.0.0".into(),
            sha256: "a".repeat(64),
            size: 4096,
            url: "modules/android-arm64/app-notes.so".into(),
            sig: "modules/android-arm64/app-notes.so.sig".into(),
        }
    }

    fn catalog() -> Catalog {
        Catalog {
            schema: SCHEMA,
            abi: "android-arm64".into(),
            generated: "2026-10-25".into(),
            modules: vec![entry()],
        }
    }

    fn expect_err(cat: &Catalog, abi: &str) -> String {
        cat.validate(abi).unwrap_err().to_string()
    }

    #[test]
    fn valid_catalog_parses() {
        let json = serde_json::to_string(&catalog()).unwrap();
        let cat = Catalog::parse(&json, "android-arm64").unwrap();
        assert_eq!(cat.entry("app-notes").unwrap().version, "1.0.0");
        assert!(cat.entry("app-missing").is_none());
    }

    #[test]
    fn empty_catalog_is_allowed_but_wrong_abi_is_not() {
        let mut cat = catalog();
        cat.modules.clear();
        cat.validate("android-arm64").unwrap();
        // An empty store is a state the UI must render ("nothing published
        // yet"), but serving the arm64 registry to an x86_64 request is a
        // misconfiguration worth failing on.
        let msg = expect_err(&cat, "android-x86_64");
        assert!(msg.contains("android-arm64"), "{msg}");
    }

    #[test]
    fn wrong_schema_and_malformed_json_are_rejected() {
        let mut cat = catalog();
        cat.schema = 99;
        assert!(expect_err(&cat, "android-arm64").contains("schema 99"));
        let msg = Catalog::parse("{ not json", "android-arm64").unwrap_err();
        assert!(msg.to_string().contains("catalog JSON"), "{msg}");
        let msg = Catalog::parse("{}", "android-arm64").unwrap_err();
        assert!(msg.to_string().contains("missing field"), "{msg}");
    }

    #[test]
    fn bad_ids_are_rejected() {
        let mut ids = vec![String::new(), "../evil".into(), "UPPER".into()];
        ids.push("app-notes/extra".into());
        ids.push("a".repeat(65));
        for id in ids {
            let mut cat = catalog();
            cat.modules[0].id = id.clone();
            let msg = expect_err(&cat, "android-arm64");
            assert!(msg.contains("invalid id"), "{id} -> {msg}");
        }
    }

    #[test]
    fn duplicate_ids_are_rejected() {
        let mut cat = catalog();
        cat.modules.push(entry());
        let msg = expect_err(&cat, "android-arm64");
        assert!(msg.contains("duplicate id"), "{msg}");
    }

    #[test]
    fn traversal_urls_are_rejected() {
        for url in [
            "/etc/passwd",
            "https://evil.example/x.so",
            "../../x.so",
            "modules/../../../x.so",
            "modules\\x.so",
            "",
        ] {
            let mut cat = catalog();
            cat.modules[0].url = url.into();
            let msg = expect_err(&cat, "android-arm64");
            assert!(msg.contains("relative path"), "{url} -> {msg}");
        }
        // Same rule applies to the signature path.
        let mut cat = catalog();
        cat.modules[0].sig = "https://evil.example/x.sig".into();
        let msg = expect_err(&cat, "android-arm64");
        assert!(msg.contains("sig must be a relative path"), "{msg}");
    }

    #[test]
    fn size_and_hash_fields_are_enforced() {
        let mut cat = catalog();
        cat.modules[0].sha256 = "zz".repeat(32);
        assert!(expect_err(&cat, "android-arm64").contains("64 hex"));
        cat.modules[0].sha256 = "a".repeat(63);
        assert!(expect_err(&cat, "android-arm64").contains("64 hex"));

        cat = catalog();
        cat.modules[0].size = 0;
        assert!(expect_err(&cat, "android-arm64").contains("outside"));
        cat.modules[0].size = MAX_MODULE_BYTES + 1;
        assert!(expect_err(&cat, "android-arm64").contains("outside"));
    }

    #[test]
    fn label_version_and_description_bounds() {
        let mut cat = catalog();
        cat.modules[0].label = "   ".into();
        assert!(expect_err(&cat, "android-arm64").contains("label"));
        cat.modules[0].label = "x".repeat(81);
        assert!(expect_err(&cat, "android-arm64").contains("label"));

        cat = catalog();
        cat.modules[0].version = "".into();
        assert!(expect_err(&cat, "android-arm64").contains("version"));

        cat = catalog();
        cat.modules[0].description = "x".repeat(401);
        assert!(expect_err(&cat, "android-arm64").contains("description"));
    }

    #[test]
    fn registry_round_trip_is_lossless() {
        let json = serde_json::to_string_pretty(&catalog()).unwrap();
        assert_eq!(Catalog::parse(&json, "android-arm64").unwrap(), catalog());
    }

    #[test]
    fn abi_helpers_map_predictably() {
        assert_eq!(abi_e_machine("android-arm64"), Some(183));
        assert_eq!(abi_e_machine("android-x86_64"), Some(62));
        assert_eq!(abi_e_machine("windows"), None);
        // Desktop: no registry is published for this platform.
        if std::env::consts::OS != "android" {
            assert_eq!(current_abi(), None);
        }
    }
}
