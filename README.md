# website (authoring branch for the module store)

Everything the module store needs that is *not* a compiled binary lives here:

- `store-site/index.html` — the store's landing page, copied verbatim onto the
  serving `store` branch by CI.
- `store/manifests/<module-id>.json` — per-module metadata (label,
  description) consumed by `mandala-sign registry` when it generates
  `registry/<abi>.json`.

Branch layout of this repository:

| branch    | contents                                            | served? |
|-----------|-----------------------------------------------------|---------|
| `main`    | launcher shell only — no module sources             | releases APK + desktop archives |
| `mod/*`   | one module per branch, own release workflow         | release tags `app-*/v*` |
| `website` | this branch: site source + module metadata          | no |
| `store`   | deployed store: index.html + registry/ + modules/   | GitHub Pages source |

Releasing a module (`mod/<id>` branch): tag `<id>/vX.Y.Z` on that branch;
its `module.yml` tests, cross-builds, signs and publishes onto `store`.

To change a module's storefront metadata, edit `store/manifests/<id>.json`
here and push this branch; the next module publish regenerates the catalogs.
