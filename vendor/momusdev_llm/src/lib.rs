//! Build **stub** for `momusdev_llm` — see [ADR-00016].
//!
//! This crate exists ONLY so the public workspace resolves and builds native-free
//! without the private `momusdev_llm` engine. Cargo resolves path dependencies
//! eagerly, so a real, present manifest is required for *any* build — even the
//! default, no-feature one — to succeed on a fresh clone.
//!
//! It is an **optional** dependency of `doctree-llm` and is **never compiled by the
//! default build** (the engine lives behind the `llm` / `vectordb` features). To
//! build those features, override this stub with the real crate on a machine that
//! has it, via Cargo's `.cargo/config.toml` `paths` key (see
//! `.cargo/config.toml.example` and ADR-00016) — the real API then replaces this
//! empty surface. Building `--features llm` *without* that override will fail to
//! compile (this stub has no API), which is the intended, clearly-signposted result.
//!
//! [ADR-00016]: ../../docs/adr/ADR-00016-momusdev-llm-build-stub.md
