# tests/ (owner: qa)

Cross-crate and end-to-end checks. Unit tests live next to the code in each crate.

Planned layout:

```
tests/
  contract/     bindings.ts drift, hub responses vs server/openapi/hub.yaml
  fixtures/     version manifests, mrpack/CF zips, loader metadata (small, committed)
  e2e/          scripted scenarios: create instance → launch (needs network, run manually / nightly)
  network/      RU-network scenarios: blocked hosts, mirrors fallback
```

qa only adds tests here and reports failures; fixes are made by the owning role.
