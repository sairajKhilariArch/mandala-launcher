# mod/app-localsend

Autonomous branch for the `app-localsend` Mandala module.

- Workspace: `crates/mandala-module-api` + `modules/app-localsend` only. Compiles and
  tests standalone; the launcher shell lives on the `main` branch.
- Release from this branch

      git switch mod/app-localsend
      git tag app-localsend/vX.Y.Z && git push origin mod/app-localsend app-localsend/vX.Y.Z

  `.github/workflows/module.yml` then runs `cargo test --workspace`, builds the
  module for `android-arm64`, `android-x86_64` and (host) `linux-x86_64`,
  signs each `app-localsend.so` with `MANDALA_SIGNING_KEY`, verifies the signatures
  against main's committed `release.pub`, regenerates `registry/<abi>.json`
  and pushes the results to the orphan `store` branch (GitHub Pages).
- Module metadata (label, description) lives on the `website` branch:
  `store/manifests/app-localsend.json`.
