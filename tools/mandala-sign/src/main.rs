//! `mandala-sign`: developer tool for the module trust chain.
//!
//! ```text
//! mandala-sign keygen  <dir> [--force]
//! mandala-sign sign    --key <host.key> <binary>
//! mandala-sign verify  --pub <host.pub> <binary>
//! mandala-sign install --home <mandala-home> --key <host.key> <binary>...
//! ```
//!
//! The signing key (`host.key`) is a secret: keep it off the device. Only the
//! public key (`host.pub`) ships in `<home>/trust/`.

use anyhow::{bail, Context, Result};
use ed25519_dalek::SigningKey;
use mandala_core::catalog::{Catalog, CatalogEntry};
use mandala_core::crypto::{
    parse_public_key, parse_signing_key, sha256, sha256_hex, sign_binary, TrustStore,
};
use mandala_core::paths::HostPaths;
use rand::rngs::OsRng;
use std::path::{Path, PathBuf};

const USAGE: &str = "usage:
  mandala-sign keygen  <dir> [--force]
  mandala-sign sign     --key <host.key> <binary>
  mandala-sign verify   --pub <host.pub> <binary>
  mandala-sign install  --home <mandala-home> --key <host.key> <binary>...
  mandala-sign registry --abi <abi> --dir <signed-modules> --manifests <dir> \\
                        --out <registry.json> [--pub <host.pub>]";

fn main() {
    if let Err(e) = run(std::env::args().skip(1).collect()) {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

fn run(args: Vec<String>) -> Result<()> {
    let Some((cmd, rest)) = args.split_first() else {
        bail!("{USAGE}")
    };
    match cmd.as_str() {
        "keygen" => keygen(rest),
        "sign" => sign(rest),
        "verify" => verify(rest),
        "install" => install(rest),
        "registry" => registry(rest),
        "-h" | "--help" | "help" => {
            println!("{USAGE}");
            Ok(())
        }
        other => bail!("unknown command `{other}`\n{USAGE}"),
    }
}

/// Pull `--name value` out of `args`; returns the value and the remaining args.
fn take_opt(args: &[String], name: &str) -> Result<(Option<String>, Vec<String>)> {
    let mut rest = Vec::new();
    let mut val = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == name {
            val = Some(
                it.next()
                    .with_context(|| format!("{name} needs a value"))?
                    .clone(),
            );
        } else {
            rest.push(a.clone());
        }
    }
    Ok((val, rest))
}

fn keygen(args: &[String]) -> Result<()> {
    let force = args.iter().any(|a| a == "--force");
    let dirs: Vec<_> = args.iter().filter(|a| *a != "--force").collect();
    let [dir] = dirs.as_slice() else {
        bail!("{USAGE}")
    };
    let dir = PathBuf::from(dir);
    std::fs::create_dir_all(&dir)?;
    let (key_path, pub_path) = (dir.join("host.key"), dir.join("host.pub"));
    if !force && (key_path.exists() || pub_path.exists()) {
        bail!(
            "{} or {} already exists (use --force to overwrite)",
            key_path.display(),
            pub_path.display()
        );
    }
    let sk = SigningKey::generate(&mut OsRng);
    write_secret(&key_path, hex::encode(sk.to_bytes()).as_bytes())?;
    std::fs::write(
        &pub_path,
        format!("{}\n", hex::encode(sk.verifying_key().to_bytes())),
    )?;
    println!("wrote {} (SECRET, keep off-device)", key_path.display());
    println!(
        "wrote {} (copy into <mandala-home>/trust/)",
        pub_path.display()
    );
    Ok(())
}

fn load_key(path: &str) -> Result<SigningKey> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {path}"))?;
    Ok(parse_signing_key(&text)?)
}

fn sig_path(binary: &Path) -> PathBuf {
    let mut p = binary.as_os_str().to_owned();
    p.push(".sig");
    PathBuf::from(p)
}

fn sign_file(key: &SigningKey, binary: &Path) -> Result<PathBuf> {
    let bytes = std::fs::read(binary).with_context(|| format!("reading {}", binary.display()))?;
    let sig = sign_binary(key, &bytes);
    let out = sig_path(binary);
    std::fs::write(&out, format!("{}\n", hex::encode(sig.to_bytes())))?;
    Ok(out)
}

fn sign(args: &[String]) -> Result<()> {
    let (key, rest) = take_opt(args, "--key")?;
    let key = load_key(&key.context("--key is required")?)?;
    let [binary] = rest.as_slice() else {
        bail!("{USAGE}")
    };
    let out = sign_file(&key, Path::new(binary))?;
    println!("signed {binary} -> {}", out.display());
    Ok(())
}

fn verify(args: &[String]) -> Result<()> {
    let (pubkey, rest) = take_opt(args, "--pub")?;
    let pubkey = pubkey.context("--pub is required")?;
    let [binary] = rest.as_slice() else {
        bail!("{USAGE}")
    };
    let pk = parse_public_key(
        &std::fs::read_to_string(&pubkey).with_context(|| format!("reading {pubkey}"))?,
    )?;
    let bytes = std::fs::read(binary).with_context(|| format!("reading {binary}"))?;
    let sig = std::fs::read(sig_path(Path::new(binary))).context("reading signature file")?;
    TrustStore::new(vec![pk]).verify(&sha256(&bytes), &sig)?;
    println!("OK: {binary} is signed by {pubkey}");
    Ok(())
}

/// `libapp_notes.so` -> `app-notes`, `app-calculator.dll` -> `app-calculator`.
fn module_id_from_filename(name: &str) -> String {
    let stem = Path::new(name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(name);
    stem.strip_prefix("lib")
        .unwrap_or(stem)
        .replace('_', "-")
        .to_ascii_lowercase()
}

fn install(args: &[String]) -> Result<()> {
    let (home, rest) = take_opt(args, "--home")?;
    let (key, rest) = take_opt(&rest, "--key")?;
    let paths = HostPaths::new(home.context("--home is required")?);
    let key = load_key(&key.context("--key is required")?)?;
    if rest.is_empty() {
        bail!("{USAGE}");
    }
    paths.ensure()?;
    for binary in &rest {
        let name = Path::new(binary)
            .file_name()
            .and_then(|n| n.to_str())
            .context("bad file name")?;
        let id = module_id_from_filename(name);
        mandala_core::loader::validate_module_id(&id)?;
        let dest = paths.module_file(&id);
        std::fs::copy(binary, &dest).with_context(|| format!("copying {binary}"))?;
        sign_file(&key, &dest)?;
        println!("installed {id} -> {}", dest.display());
    }
    Ok(())
}

#[derive(serde::Deserialize)]
struct Manifest {
    id: String,
    label: String,
    #[serde(default)]
    description: String,
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Scan a directory of signed binaries into the catalog the launcher fetches.
///
/// The `store` branch keeps `modules/<abi>/<id>.so`, `<id>.so.sig` and
/// `<id>.so.ver` (the release version) side by side; display copy lives in
/// `store/manifests/<id>.json` on main. The result is validated with the same
/// `Catalog` rules the launcher enforces, so CI cannot publish a catalog the
/// Store would refuse.
fn build_registry(
    abi: &str,
    dir: &Path,
    manifests: &Path,
    pubkey: Option<&Path>,
) -> Result<Catalog> {
    if mandala_core::catalog::abi_e_machine(abi).is_none() {
        bail!("unknown ABI `{abi}` (expected android-arm64 or android-x86_64)");
    }
    let trust = match pubkey {
        Some(p) => {
            let text =
                std::fs::read_to_string(p).with_context(|| format!("reading {}", p.display()))?;
            Some(TrustStore::new(vec![parse_public_key(&text)?]))
        }
        None => None,
    };

    let mut modules = Vec::new();
    let entries = std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("so") {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .with_context(|| format!("bad file name {}", path.display()))?
            .to_string();
        let id = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
        mandala_core::loader::validate_module_id(&id)
            .with_context(|| format!("module id from {}", path.display()))?;

        let bytes = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
        let sig_path = sig_path(&path);
        let sig = std::fs::read(&sig_path)
            .with_context(|| format!("missing signature {}", sig_path.display()))?;
        if let Some(ts) = &trust {
            ts.verify(&sha256(&bytes), &sig)
                .with_context(|| format!("signature check for {id}"))?;
        }

        let ver_path = path.with_file_name(format!("{name}.ver"));
        let version = std::fs::read_to_string(&ver_path)
            .with_context(|| format!("missing version file {}", ver_path.display()))?
            .trim()
            .to_string();

        let manifest_path = manifests.join(format!("{id}.json"));
        let text = std::fs::read_to_string(&manifest_path)
            .with_context(|| format!("missing manifest {}", manifest_path.display()))?;
        let manifest: Manifest = serde_json::from_str(&text)
            .with_context(|| format!("manifest {}", manifest_path.display()))?;
        if manifest.id != id {
            bail!(
                "{} says id `{}` but the binary is `{id}`",
                manifest_path.display(),
                manifest.id
            );
        }

        modules.push(CatalogEntry {
            id,
            label: manifest.label,
            description: manifest.description,
            version,
            sha256: sha256_hex(&bytes),
            size: bytes.len() as u64,
            url: format!("modules/{abi}/{name}"),
            sig: format!("modules/{abi}/{name}.sig"),
        });
    }
    if modules.is_empty() {
        bail!("no signed modules in {}", dir.display());
    }
    modules.sort_by(|a, b| a.id.cmp(&b.id));
    let cat = Catalog {
        schema: mandala_core::catalog::SCHEMA,
        abi: abi.to_string(),
        generated: format!("unix:{}", now_secs()),
        modules,
    };
    cat.validate(abi)?;
    Ok(cat)
}

fn registry(args: &[String]) -> Result<()> {
    let (abi, rest) = take_opt(args, "--abi")?;
    let (dir, rest) = take_opt(&rest, "--dir")?;
    let (manifests, rest) = take_opt(&rest, "--manifests")?;
    let (out, rest) = take_opt(&rest, "--out")?;
    let (pubkey, rest) = take_opt(&rest, "--pub")?;
    if !rest.is_empty() {
        bail!("{USAGE}");
    }
    let abi = abi.context("--abi is required")?;
    let dir = PathBuf::from(dir.context("--dir is required")?);
    let manifests = PathBuf::from(manifests.context("--manifests is required")?);
    let out = PathBuf::from(out.context("--out is required")?);

    let cat = build_registry(&abi, &dir, &manifests, pubkey.as_deref().map(Path::new))?;
    if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&out, format!("{}\n", serde_json::to_string_pretty(&cat)?))?;
    println!(
        "wrote {} ({} modules, abi {abi})",
        out.display(),
        cat.modules.len()
    );
    Ok(())
}

#[cfg(unix)]
fn write_secret(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(bytes)?;
    f.write_all(b"\n")?;
    Ok(())
}

#[cfg(not(unix))]
fn write_secret(path: &Path, bytes: &[u8]) -> Result<()> {
    std::fs::write(path, [bytes, b"\n"].concat())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn filename_to_module_id() {
        assert_eq!(module_id_from_filename("libapp_notes.so"), "app-notes");
        assert_eq!(
            module_id_from_filename("app_calculator.dll"),
            "app-calculator"
        );
        assert_eq!(module_id_from_filename("libapp_aegis.dylib"), "app-aegis");
    }

    #[test]
    fn keygen_sign_verify_install_roundtrip() {
        let d = tempfile::tempdir().unwrap();
        let keys = d.path().join("keys");
        run(s(&["keygen", keys.to_str().unwrap()])).unwrap();
        assert!(
            run(s(&["keygen", keys.to_str().unwrap()])).is_err(),
            "must not overwrite"
        );
        run(s(&["keygen", keys.to_str().unwrap(), "--force"])).unwrap();

        let bin = d.path().join("libapp_notes.so");
        std::fs::write(&bin, b"pretend module bytes").unwrap();
        let key = keys.join("host.key");
        let pubf = keys.join("host.pub");
        run(s(&[
            "sign",
            "--key",
            key.to_str().unwrap(),
            bin.to_str().unwrap(),
        ]))
        .unwrap();
        run(s(&[
            "verify",
            "--pub",
            pubf.to_str().unwrap(),
            bin.to_str().unwrap(),
        ]))
        .unwrap();

        std::fs::write(&bin, b"tampered").unwrap();
        assert!(run(s(&[
            "verify",
            "--pub",
            pubf.to_str().unwrap(),
            bin.to_str().unwrap()
        ]))
        .is_err());

        std::fs::write(&bin, b"pretend module bytes").unwrap();
        let home = d.path().join("home");
        run(s(&[
            "install",
            "--home",
            home.to_str().unwrap(),
            "--key",
            key.to_str().unwrap(),
            bin.to_str().unwrap(),
        ]))
        .unwrap();
        let installed = HostPaths::new(&home).module_file("app-notes");
        assert!(installed.exists());
        run(s(&[
            "verify",
            "--pub",
            pubf.to_str().unwrap(),
            installed.to_str().unwrap(),
        ]))
        .unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn secret_key_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        run(s(&["keygen", d.path().to_str().unwrap()])).unwrap();
        let mode = std::fs::metadata(d.path().join("host.key"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }

    fn write_module(dir: &Path, id: &str, version: &str, key: &SigningKey) {
        std::fs::create_dir_all(dir).unwrap();
        let so = dir.join(format!("{id}.so"));
        std::fs::write(&so, format!("binary of {id}")).unwrap();
        sign_file(key, &so).unwrap();
        std::fs::write(dir.join(format!("{id}.so.ver")), format!("{version}\n")).unwrap();
    }

    fn write_manifest(dir: &Path, id: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(
            dir.join(format!("{id}.json")),
            format!(r#"{{"id":"{id}","label":"Label {id}","description":"Desc {id}"}}"#),
        )
        .unwrap();
    }

    #[test]
    fn registry_builds_and_validates_a_catalog() {
        let d = tempfile::tempdir().unwrap();
        let keys = d.path().join("keys");
        run(s(&["keygen", keys.to_str().unwrap()])).unwrap();
        let key = load_key(keys.join("host.key").to_str().unwrap()).unwrap();
        let pubf = keys.join("host.pub");

        let mods = d.path().join("modules/android-arm64");
        let mans = d.path().join("manifests");
        write_module(&mods, "app-weather", "0.9.1", &key);
        write_module(&mods, "app-notes", "1.2.0", &key);
        write_manifest(&mans, "app-weather");
        write_manifest(&mans, "app-notes");

        let cat = build_registry("android-arm64", &mods, &mans, Some(&pubf)).unwrap();
        assert_eq!(cat.modules.len(), 2);
        assert_eq!(cat.modules[0].id, "app-notes"); // sorted by id
        assert_eq!(cat.modules[0].version, "1.2.0");
        assert_eq!(cat.modules[0].url, "modules/android-arm64/app-notes.so");
        assert_eq!(cat.modules[0].size, "binary of app-notes".len() as u64);
        // The launcher parses exactly this shape.
        let json = serde_json::to_string(&cat).unwrap();
        let back = Catalog::parse(&json, "android-arm64").unwrap();
        assert_eq!(back, cat);

        // A tampered binary fails verification before anything is published.
        std::fs::write(mods.join("app-notes.so"), b"tampered").unwrap();
        assert!(build_registry("android-arm64", &mods, &mans, Some(&pubf)).is_err());
    }

    #[test]
    fn registry_rejects_incomplete_or_mismatched_inputs() {
        let d = tempfile::tempdir().unwrap();
        let keys = d.path().join("keys");
        run(s(&["keygen", keys.to_str().unwrap()])).unwrap();
        let key = load_key(keys.join("host.key").to_str().unwrap()).unwrap();
        let mods = d.path().join("modules/android-arm64");
        let mans = d.path().join("manifests");
        write_module(&mods, "app-notes", "1.0.0", &key);
        write_manifest(&mans, "app-notes");

        // Missing manifest.
        std::fs::remove_file(mans.join("app-notes.json")).unwrap();
        assert!(build_registry("android-arm64", &mods, &mans, None).is_err());
        write_manifest(&mans, "app-notes");

        // Missing version sidecar.
        std::fs::remove_file(mods.join("app-notes.so.ver")).unwrap();
        assert!(build_registry("android-arm64", &mods, &mans, None).is_err());
        std::fs::write(mods.join("app-notes.so.ver"), "1.0.0\n").unwrap();

        // Missing signature.
        std::fs::remove_file(mods.join("app-notes.so.sig")).unwrap();
        assert!(build_registry("android-arm64", &mods, &mans, None).is_err());
        sign_file(&key, &mods.join("app-notes.so")).unwrap();

        // Manifest id that disagrees with the file name.
        std::fs::write(
            mans.join("app-notes.json"),
            r#"{"id":"app-other","label":"x","description":""}"#,
        )
        .unwrap();
        assert!(build_registry("android-arm64", &mods, &mans, None).is_err());
        write_manifest(&mans, "app-notes");

        // Unknown ABI and an empty directory are both hard errors.
        assert!(build_registry("windows", &mods, &mans, None).is_err());
        let empty = d.path().join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        assert!(build_registry("android-arm64", &empty, &mans, None).is_err());
    }

    #[test]
    fn registry_cli_writes_the_catalog_file() {
        let d = tempfile::tempdir().unwrap();
        let keys = d.path().join("keys");
        run(s(&["keygen", keys.to_str().unwrap()])).unwrap();
        let key = load_key(keys.join("host.key").to_str().unwrap()).unwrap();
        let mods = d.path().join("modules/android-arm64");
        let mans = d.path().join("manifests");
        write_module(&mods, "app-notes", "1.0.0", &key);
        write_manifest(&mans, "app-notes");
        let out = d.path().join("registry/android-arm64.json");

        run(s(&[
            "registry",
            "--abi",
            "android-arm64",
            "--dir",
            mods.to_str().unwrap(),
            "--manifests",
            mans.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
            "--pub",
            keys.join("host.pub").to_str().unwrap(),
        ]))
        .unwrap();

        let text = std::fs::read_to_string(&out).unwrap();
        let cat = Catalog::parse(&text, "android-arm64").unwrap();
        assert_eq!(cat.modules.len(), 1);
        assert_eq!(cat.modules[0].id, "app-notes");
    }

    #[test]
    fn bad_usage_errors() {
        assert!(run(vec![]).is_err());
        assert!(run(s(&["wat"])).is_err());
        assert!(run(s(&["sign", "x"])).is_err());
    }
}
