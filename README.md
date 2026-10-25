# mod/app-vault

Autonomous branch for the `app-vault` Mandala module.

- Workspace: `crates/mandala-module-api` + `modules/app-vault` only. Compiles and
  tests standalone; the launcher shell lives on the `main` branch.
- Release from this branch

      git switch mod/app-vault
      git tag app-vault/vX.Y.Z && git push origin mod/app-vault app-vault/vX.Y.Z

  `.github/workflows/module.yml` then runs `cargo test --workspace`, builds the
  module for `android-arm64`, `android-x86_64` and (host) `linux-x86_64`,
  signs each `app-vault.so` with `MANDALA_SIGNING_KEY`, verifies the signatures
  against main's committed `release.pub`, regenerates `registry/<abi>.json`
  and pushes the results to the orphan `store` branch (GitHub Pages).
- Module metadata (label, description) lives on the `website` branch:
  `store/manifests/app-vault.json`.
