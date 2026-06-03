# ADR-00016 — In-repo build stub for the private `momusdev_llm` engine

- **Status:** Accepted (2026-06-03)
- **Phase:** 8 (#13)
- **Deciders:** Travis (user), agent
- **Depends on:** [ADR-0001](ADR-0001-stack-tauri-rust-web3d.md),
  [ADR-00014](ADR-00014-zero-config-model-discovery.md)
- **Supersedes / superseded by:** none

## Context

`doctree-llm` consumes the local-LLM engine `momusdev_llm` as an **optional** Cargo
dependency. Until now that dependency was an **absolute machine path**
(`path = "E:/Development/momusdev-packages/momusdev_llm"`). Two problems for a public
repo (the user pushed the repo to GitHub):

1. **The public repo did not build at all — not even native-free.** Cargo resolves
   path dependencies **eagerly**: it loads a path dep's manifest while loading the
   workspace, *before* feature selection. Verified by pointing the dep at a
   non-existent path: `cargo metadata` (default, no features) failed with
   `error: failed to load manifest for dependency momusdev_llm` (exit 101). So a
   fresh clone on any machine without that exact path was unbuildable.
2. **The committed manifest leaked a machine path** (`E:/Development/momusdev-packages/…`).

Constraints: `momusdev_llm` is the user's **private, unpublished** crate, consumed
**read-only** and **never modified** (ADR-0001). Its source must not be published,
and Rust has no portable compiled-library distribution (no stable ABI; an `.rlib` is
compiler-version-specific), so "ship a momusdev_llm binary to link against" is not a
real option for a source build.

## Decision

Add an **in-repo build stub** for `momusdev_llm` and select the real engine locally
via a Cargo `paths` override.

1. **Stub crate `vendor/momusdev_llm/`** — package `name = "momusdev_llm"`,
   `version = "0.1.0"`, an empty `lib.rs`, and the feature names `doctree-llm`
   cascades to (`inference`, `cuda`, `vulkan`, `vectordb`) declared as no-ops so
   feature resolution stays valid. It is **excluded from the workspace**
   (`exclude = ["vendor/momusdev_llm"]`) — a dependency, not a project crate.
2. **`doctree-llm` points its optional `momusdev_llm` dep at the in-repo stub**
   (`path = "../../vendor/momusdev_llm"`) — a relative, always-present path, so a
   fresh clone resolves and the **default (no-feature) build is native-free** (the
   stub is optional and never compiled by the default build). No machine path in the
   committed manifest.
3. **Local override for the real engine** — Cargo's `.cargo/config.toml` `paths` key
   replaces the stub (same name + version) with the real crate on a machine that has
   it. The real `.cargo/config.toml` is **gitignored** (it carries the machine path);
   a committed **`.cargo/config.toml.example`** documents the pattern. With the
   override active, `--features llm,vectordb` compiles the real backends.

## Alternatives considered

1. **Publish `momusdev_llm` to crates.io.** Rejected — publishes the private source.
2. **Private git / private registry dependency.** Rejected — optional deps from those
   sources are *also* resolved eagerly, so public clones without access still fail the
   default build; and it is still distributing source, just access-gated.
3. **Ship a prebuilt `momusdev_llm` Rust library.** Not viable — Rust has no stable
   ABI / portable `.rlib`. (The shipped **desktop `.exe`**, which statically links the
   compiled engine, *is* the practical "binary" — distributed via GitHub Releases —
   but that is an end-user-distribution decision, separate from making the **source**
   repo build.)
4. **A full API-matching stub** (mirror every `momusdev_llm` type/function). Rejected —
   heavy and brittle. The empty stub suffices: the default build never compiles it,
   and LLM builds use the override (the real API). Building `--features llm` *without*
   the override fails to compile — the intended, signposted result.

## Consequences

- (+) **The public repo builds native-free from a clean clone** — verified:
  with the override removed, `cargo check -p doctree-core` finishes and
  `cargo tree -p doctree-llm --features llm` resolves the **stub**; with the override,
  it resolves the **real** `momusdev_llm` at `E:/Development/momusdev-packages/…`.
- (+) **No machine path or private source in the committed manifest.**
- (+) **The LLM/embedding workflow is unchanged** for anyone who has the engine and the
  one-line local override (the user's `scripts/build-desktop.cmd` keeps working).
- (−) `cargo build --features llm` **without** the override fails to compile (the stub
  has no API) — by design, pointed at the example/ADR.
- (−) Cargo prints a benign `path override … has altered the original list of
  dependencies` advisory when the override pulls the real crate's extra deps
  (`momusdev_met`). Harmless.
- (−) One vendored stub to keep in sync **only if** `momusdev_llm`'s *feature names*
  change (not its API).

## Invariant (pinned)

The public-clone guarantee is a build invariant, checked by simulating a fresh clone
(no `.cargo/config.toml` override, stub only):

- `cargo metadata` / `cargo check -p doctree-core` (default, no features) **succeed**
  with only the in-repo stub present — no real engine, no machine path.
- `cargo tree -p doctree-llm --features llm` resolves `momusdev_llm` to
  `vendor/momusdev_llm` without the override, and to the real crate with it.
- The standing `cargo test --workspace` (no features) stays green and native-free.

## Anchors

- `vendor/momusdev_llm/` — the stub crate (`Cargo.toml`, `src/lib.rs`).
- `crates/doctree-llm/Cargo.toml` — `momusdev_llm` optional dep → `../../vendor/momusdev_llm`.
- `Cargo.toml` — `[workspace] exclude = ["vendor/momusdev_llm"]`.
- `.cargo/config.toml.example` — the `paths` override template; the real
  `.cargo/config.toml` is gitignored.
- `README.md` — the external-builder note.
