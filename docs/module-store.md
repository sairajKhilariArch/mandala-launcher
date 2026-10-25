# Module store

The launcher ships as a shell. Modules are signed shared libraries published
separately, downloaded by the in-app store, verified against a key baked into
the launcher, and installed into app-private storage — no APK rebuild per
module.

```
                      GitHub Actions (module.yml)
  app-notes/v1.2.0 ──► build arm64+x86_64 ──► sign ──► registry ──► `store` branch
                                                                        │
                                                     GitHub Pages (HTTPS)
                                                                        │
  ┌────────────────────── launcher on the device ────────────────────────┴──┐
  │ Store overlay ─► catalog fetch ─► download ─► SHA-256 + Ed25519 verify  │
  │                      │                                   │              │
  │                 cache on disk                    <home>/modules/<id>.so │
  │                                                  <home>/modules/<id>.so.sig
  └─────────────────────────────────────────────────────────────────────────┘
```

## One-time setup

1. **Generate the release key** (once; never commit the private half):

   ```bash
   cargo run --release -p mandala-sign -- keygen release-keys
   ```

2. **Bake the public key into the launcher.** Copy `release-keys/host.pub` to
   `crates/mandala-shell/release.pub` and commit it. The shell reads that file
   at compile time, so every build trusts exactly that key. Until you do this,
   the shell warns on startup that it has no trusted key and the store refuses
   to install anything (correct behaviour — an unsigned module must never load).

3. **Add the CI secret.** Repository → Settings → Secrets → `MANDALA_SIGNING_KEY`
   = the contents of `release-keys/host.key` (hex). Used by `release.yml` to sign
   the desktop bundle and by `module.yml` to sign published modules.

4. **Enable Pages.** Settings → Pages → Source = *Deploy from a branch* →
   branch `store`, folder `/ (root)`. The first `module.yml` run creates that
   orphan branch.

5. Push a tag: `git tag app-notes/v1.2.0 && git push origin app-notes/v1.2.0`.

## What a module release does

`.github/workflows/module.yml`, one module per tag (`<id>/v<semver>`):

1. validates the module id and version **before** building,
2. `cargo ndk` builds it for `arm64-v8a` and `x86_64`,
3. renames `libapp_notes.so` → `app-notes.so` (the loader's id convention),
4. signs it → `app-notes.so.sig`, and writes `app-notes.so.ver`,
5. copies it onto the `store` branch under `modules/<abi>/`,
6. regenerates `registry/<abi>.json` from **all** modules on the branch,
   verifying each signature against `crates/mandala-shell/release.pub` — a
   mismatched secret fails here instead of shipping modules devices reject,
7. commits and pushes. Pages serves the result at
   `https://<owner>.github.io/<repo>/`.

`main` never publishes module bytes: `release.yml` ships the APK/desktop shell
plus a developer-only signed `mandala-home/`, and `module.yml` owns everything
else. Both workflows require `MANDALA_SIGNING_KEY`; neither can publish
unsigned modules.

## Catalog format

`registry/android-arm64.json` (and the `android-x86_64.json` sibling):

```json
{
  "schema": 1,
  "generated": "2026-10-25T09:12:44Z",
  "abi": "android-arm64",
  "modules": [
    {
      "id": "app-notes",
      "label": "Notes",
      "description": "Append-only text buffer",
      "version": "1.2.0",
      "sha256": "6b1f…64 hex chars…",
      "size": 284160,
      "url": "modules/android-arm64/app-notes.so",
      "sig": "modules/android-arm64/app-notes.so.sig"
    }
  ]
}
```

`url` and `sig` must be **site-relative**: no scheme, no leading `/`, no `..`.
A catalog that tries to point downloads elsewhere is rejected wholesale rather
than silently fetching from a foreign host. Parse and validation rules live in
[`crates/mandala-core/src/catalog.rs`](../crates/mandala-core/src/catalog.rs)
with tests (schema, duplicate ids, id charset, field lengths, path escapes).

The base URL is resolved by `Store::new` in this order:

1. `MANDALA_STORE_URL` (desktop/dev),
2. `<home>/store.url` (on-device override — app-private, and a tampered value
   can only redirect downloads that still must pass the signature check),
3. `https://sairajkhilariarch.github.io/mandala-launcher` (the constant in
   `crates/mandala-shell/src/store.rs`).

The registry is then `{base}/registry/{abi}.json`, where `abi` comes from the
running platform (`android-arm64`, `android-x86_64`). Desktop builds have no
published registry and the store says so instead of showing an empty list.

## Install flow (what the shell enforces)

`crates/mandala-shell/src/store.rs`, all blocking work on the blocking pool:

1. **catalog** — `GET` the registry (15 s timeout, 512 KiB cap), parse and
   validate, keep the validated copy in `<home>/data/.store/catalog-<abi>.json`
   for offline browsing. A served-but-*invalid* catalog is an error, not a
   silent fallback — the cache only covers "the network failed".
2. **download** — `GET` the binary (cap 64 MiB, and it must be *exactly* the
   catalog's `size`) and the signature (cap 4 KiB).
3. **verify** — SHA-256 equals the catalog `sha256`; the first four bytes are
   `\x7fELF`; the ELF `e_machine` matches the device ABI (183 / 62); the
   Ed25519 signature verifies against the key baked into the shell at compile
   time. Nothing is written to `modules/` before all four pass.
4. **install** — stage to `<home>/data/.store/<id>.so.part` + `.sig.part`, then
   `rename` both into `<home>/modules/` (same filesystem, so the swap is
   atomic per file). The previous bytes are overwritten; the loader quarantines
   a signature/binary mismatch and a re-install repairs it. The cached module
   is `unload`ed and its quarantine verdict cleared so the next launch runs the
   bytes that were just verified.
5. **refresh** — the overlay's rows are recomputed by hashing the installed
   file, so a freshly installed module shows `INSTALLED ✓` immediately and the
   slot resolves with no app restart.

## Using it

Type `store` in the command bar (or pick **S Store** in the All Apps drawer):

* the overlay opens and fetches the catalog,
* each row shows the label, version, description and one button —
  `INSTALL`, `UPDATE`, `INSTALLING…` or `INSTALLED ✓`,
* the overlay's Refresh button re-fetches the catalog,
* the status line at the top carries the offline note or the last error
  (`app-notes: signature rejected: …`), and a failure never changes another
  row's state or the installed file.

The status of a row is derived, not bookkept: the shell hashes the file in
`modules/` and compares it to the catalog, so there is no separate version
record to drift out of sync.

## Storage on Android

The launcher home is resolved by `mandala-core/src/paths.rs`:

1. `MANDALA_HOME` if set,
2. Android: `/data/data/<package>/files/mandala-home` (`internal_data_path`),
3. desktop: `$XDG_DATA_HOME/mandala-home`, else `~/.local/share/mandala-home`.

`/sdcard` is deliberately not used — it is not writable on API 30+ without
`MANAGE_EXTERNAL_STORAGE`, and module files must live in app-private storage.

To inspect it:

```bash
adb shell run-as org.mandala.launcher ls -l files/mandala-home/modules
adb logcat -s mandala
```

The shell logs the resolved home path, trusted-key count and module count on
startup, so a "no modules" screen can be diagnosed from logcat alone.

## Security properties and limits

* The signature is the trust boundary. A loaded module runs in the launcher
  process with the launcher's privileges; the IPC ACLs govern message routing,
  not memory or syscall isolation. Only install modules you would run as the
  shell itself.
* The private signing key never leaves CI secrets / your machine.
  `release-keys/`, `dev-keys/` and `*.key` are gitignored.
* The catalog is advisory: hashes and signatures decide. Tampering with
  `registry.json` on the CDN cannot install a binary that was not signed by
  the release key, and it cannot change *where* bytes are fetched from
  (entries are site-relative). A module's self-reported id is trusted, as with
  any signed code: the signature, not the catalog, is the boundary.
* Rollback is not prevented — a signed older version can be installed if the
  catalog lists it. Pin the registry you trust (`store url`) if that matters.
* Overlay/OS-level sandboxing: not provided. See
  [README_right_now.md](../README_right_now.md) for the full threat model.

## Troubleshooting

| Symptom | Cause / fix |
|---|---|
| "no trusted key" warning on startup | `crates/mandala-shell/release.pub` is missing/empty; do setup step 2 |
| Store shows "catalog: fetch …" | no network, or Pages not enabled on the `store` branch |
| `signature rejected` | the published `.so` was signed with a different key than `release.pub`; check `MANDALA_SIGNING_KEY` |
| `sha256 mismatch` | the catalog and the binary disagree — re-run the module release |
| `module id mismatch` | the binary's `__mandala_module_id` is not the catalog id; check the crate's `id()` |
| Install succeeds but the slot stays "not installed" | run `store refresh`; check logcat for the loader's rejection reason |

## Developer loop without CI

```bash
scripts/dev-setup.sh                     # dev key, signs + installs all modules locally
./target/release/mandala-shell --selftest # verifies + dlopens everything headless
```

For a local registry, generate one into `store-site/` and serve it:

```bash
M=store-site/modules/android-arm64
mkdir -p "$M" store-site/registry
cp target/release/libapp_notes.so "$M/app-notes.so"
./target/release/mandala-sign sign --key release-keys/host.key "$M/app-notes.so"
printf '1.2.0\n' > "$M/app-notes.so.ver"
./target/release/mandala-sign registry --abi android-arm64 --dir "$M" \
  --manifests store/manifests --out store-site/registry/android-arm64.json \
  --pub crates/mandala-shell/release.pub
(cd store-site && python3 -m http.server 8000)
```

Then start the launcher with `MANDALA_STORE_URL=http://127.0.0.1:8000`. Only the
*base* may be plain `http://` for local development; catalog entries stay
site-relative in every case.
