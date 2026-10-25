# mod/app-ai

Autonomous branch for the `app-ai` Mandala module.

- Workspace: `crates/mandala-module-api` + `modules/app-ai` only. Compiles and
  tests standalone; the launcher shell lives on the `main` branch.
- Release from this branch

      git switch mod/app-ai
      git tag app-ai/vX.Y.Z && git push origin mod/app-ai app-ai/vX.Y.Z

  `.github/workflows/module.yml` then runs `cargo test --workspace`, builds the
  module for `android-arm64`, `android-x86_64` and (host) `linux-x86_64`,
  signs each `app-ai.so` with `MANDALA_SIGNING_KEY`, verifies the signatures
  against main's committed `release.pub`, regenerates `registry/<abi>.json`
  and pushes the results to the orphan `store` branch (GitHub Pages).
- Module metadata (label, description) lives on the `website` branch:
  `store/manifests/app-ai.json`.
