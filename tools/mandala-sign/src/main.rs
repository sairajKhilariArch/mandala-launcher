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
use mandala_core::crypto::{parse_public_key, parse_signing_key, sha256, sign_binary, TrustStore};
use mandala_core::paths::HostPaths;
use rand::rngs::OsRng;
use std::path::{Path, PathBuf};

const USAGE: &str = "usage:
  mandala-sign keygen  <dir> [--force]
  mandala-sign sign    --key <host.key> <binary>
  mandala-sign verify  --pub <host.pub> <binary>
  mandala-sign install --home <mandala-home> --key <host.key> <binary>...";

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

    #[test]
    fn bad_usage_errors() {
        assert!(run(vec![]).is_err());
        assert!(run(s(&["wat"])).is_err());
        assert!(run(s(&["sign", "x"])).is_err());
    }
}
