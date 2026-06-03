# Contributing

Thanks for your interest. This is a personal research project built under a strict
*substrate-first* discipline; contributions are welcome but should follow the same
conventions so the history stays legible.

## Ground rules

- **Decisions before code.** Any load-bearing choice gets an **ADR** in `docs/adr/`
  (decision + alternatives + the invariant it pins). ADRs are immutable — supersede
  with a new one rather than editing.
- **Test-first for fixes.** A bug fix starts with a failing test that demonstrates
  the bug, then the fix that makes it pass.
- **Keep the default build native-free.** `cargo test --workspace` (no features) must
  pass with **zero** native / LLM / GPU dependencies — that decoupling is load-bearing
  (`docs/adr/ADR-0001`). Anything touching the local model lives behind the `llm` /
  `vectordb` / `cuda` feature flags.
- **Atomic commits** with a clear subject; the **CHANGELOG** entry explains *why*.

## Getting set up

```bash
npm install
cargo test --workspace     # native-free gate
npm run build              # tsc + vite
npm run tauri dev          # desktop app
```

The optional local-LLM features require the `momusdev_llm` crate (not bundled) and a
C++ toolchain; see the README.

## Reporting issues

Open an issue with the document type / size, what you expected, what happened, and —
if a graph looks wrong — the smallest input that reproduces it.
