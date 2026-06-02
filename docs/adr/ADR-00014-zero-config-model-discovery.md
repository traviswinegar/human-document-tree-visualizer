# ADR-00014 — Zero-config local-model auto-discovery

- **Status:** Accepted (2026-06-02)
- **Phase:** 7 (#5)
- **Deciders:** Travis (user), agent
- **Depends on:** [ADR-0001](ADR-0001-stack-tauri-rust-web3d.md),
  [ADR-0008](ADR-0008-gpu-cuda-acceleration.md)
- **Supersedes / superseded by:** none

## Context

The user wants **a single double-clickable release `.exe` with FULL capabilities
(LLM semantic + embeddings)** that runs with *"no PATH, no special whatevers,
unless the app manages it itself."* Two things stood between that and the current
build:

1. **The frontend.** A `target/debug/` binary loads its UI from the Vite dev
   server (`build.devUrl`) and shows a blank window when double-clicked with no
   server running. The fix is simply to ship a **release** build
   (`tauri build`), which runs `beforeBuildCommand` (`npm run build` → `dist/`)
   and embeds the assets. `--no-bundle` yields just the standalone
   `target/release/doctree-tauri.exe` (the "single exe to click"), no installer.

2. **The model.** The semantic engine needs a GGUF model on disk. Today the path
   comes **only** from the `DOCTREE_MODEL_PATH` env var that `dev-desktop.cmd`
   sets — so a bare double-click with no env gets *no model* and silently drops to
   structural-only. That violates "FULL capabilities … unless the app manages it
   itself." The embedder (vectordb / fastembed) is **already** self-managing: it
   auto-downloads the all-MiniLM-L6-v2 ONNX model into a temp-dir cache
   ([`embed_cache_dir`], ADR-0004). The LLM model is the one piece still requiring
   manual configuration.

This ADR covers (2): how the app **finds its GGUF model with zero configuration**.
The build-mode decision (release `--no-bundle`, **CPU** `llm,vectordb` rather than
`cuda`) is recorded here as context but follows directly from "single exe, no
special whatevers": CUDA needs VS 2019 + CUDA runtime DLLs alongside the exe
(ADR-0008), which is exactly the "special whatevers" the user ruled out. CPU
inference is fully self-contained.

Constraints (standing):
- **ADR-0001 decoupling.** Discovery is pure `std::fs`/`std::env` scanning. It
  lives in **`doctree-llm`** but uses **no native deps**, so it compiles and is
  unit-tested under the default `cargo test --workspace` (no features) — the
  resolution *policy* must never need the `llm`/`vectordb` build to be verified.
- It must not **hardcode a user name** or other machine-fragile absolute path; any
  per-user location is derived from an environment variable (`%LOCALAPPDATA%`).

## Decision

Add a native-free **model auto-discovery** fallback to `doctree-llm` and wire it
into `LlmConfig::resolve_model_path()` as the **last** resort.

### 1. Resolution precedence (explicit > env > discovery)

`resolve_model_path()` now resolves in this order, returning the first hit:

1. **`LlmConfig.model_path`** — an explicit path set in code (tests, future UI).
2. **`DOCTREE_MODEL_PATH`** env var, if set and non-empty (unchanged; the existing
   `dev-desktop.cmd` flow and power users keep working).
3. **`discover_model_path()`** — scan known locations for a `.gguf` (the new
   zero-config path). Only if *all three* miss does it error.

The precedence logic is factored into a pure `resolve_with(env, discovered)`
helper so it is unit-testable **without** mutating process env vars (which is
`unsafe`/racy under the 2024 edition) and **without** depending on what model
files happen to exist on the build machine.

### 2. Search path (portable-first)

`discover_model_path()` scans these directories **in priority order** and returns
the first usable `.gguf`:

1. **Next to the executable:** `<exe_dir>/` and `<exe_dir>/models/`. **This is the
   portable answer** — drop a `.gguf` beside the binary (or in a `models/`
   subfolder) and it just works: no env, no install, no PATH. A future packaging
   step can ship the model here.
2. **This app's own per-user data dir:** `%LOCALAPPDATA%/human-document-tree/models/`
   — where an in-app downloader would place the model (future work).
3. **The known momusdev/webforge model cache on this machine**, derived from
   `%LOCALAPPDATA%` (not a hardcoded user name):
   `%LOCALAPPDATA%/Packages/Claude_pzs8sxrjxfjjc/LocalCache/Roaming/com.example/webforge/webforge/models/`.
   This is where the qwen GGUFs already live (BUILD_LOG Phase 0), so the user's
   current machine gets full semantics on first double-click with **nothing** to
   set up.

Within each directory, discovery prefers known filenames in descending priority —
`qwen3-4b-q4km.gguf` (the speed/quality default), then `qwen3.5-9b-q4km.gguf`,
then `qwen2.5-0.5b-q4km.gguf` — and if none of those are present, falls back to
the directory's first `.gguf` by sorted name (deterministic). The scan helper
`discover_model_in(dirs, preferred)` is pure over its arguments, so the whole
policy is exercised in unit tests against temp dirs.

## Alternatives considered

1. **Keep env-var-only; document the setup.** Rejected: it *is* the "PATH / special
   whatevers" the user explicitly refused. The whole point is a bare double-click.
2. **Hardcode the single known qwen path.** Rejected: machine-fragile (embeds a
   user name), not portable, and offers no story for shipping the model with the
   app. Discovery subsumes it as one *derived* search dir without the fragility.
3. **Bundle the 2.3 GB GGUF inside the exe / installer.** Rejected for v1: bloats
   the artifact enormously, and `--no-bundle` is specifically the lightweight
   "single exe" the user asked for. The exe-relative search dir leaves the door
   open to ship the model *next to* the exe later without re-architecting.
4. **Auto-download the GGUF on first run** (like the embedder does its ONNX model).
   Deferred, not rejected: it is the natural next step (search dir #2 is reserved
   for it), but a 2.3 GB download with progress/retry/space-checks is its own piece
   of work. Discovery of an already-present model ships the capability now.
5. **CUDA release build** for max speed. Rejected for the *single-exe* target: it
   requires VS 2019 + CUDA runtime DLLs travelling with the exe (ADR-0008) =
   "special whatevers." CPU `llm,vectordb` is self-contained. A GPU build remains
   available via the existing `dev-desktop-gpu.cmd` path for power users.

## Consequences

- (+) **Zero-config full capability:** on a machine that already has a qwen GGUF
  (the user's), a bare double-click of the release exe loads the LLM semantic
  layer with **no** env var, PATH, or install step — meeting the request.
- (+) **Portable:** dropping a `.gguf` next to the exe (or in `<exe>/models/`)
  makes any machine work, enabling a future "ship the model alongside the exe"
  package with no code change.
- (+) **Native-free policy:** resolution precedence + the directory scan are pure
  fs/env and fully unit-tested under `cargo test --workspace` (no features) — the
  ADR-0001 decoupling holds; a broken native build never hides a resolution bug.
- (+) **Backwards-compatible:** explicit `model_path` and `DOCTREE_MODEL_PATH`
  still win, in that order, so the dev launcher and power users are unchanged.
- (−) **Discovery embeds a known package subpath** (`Claude_pzs8sxrjxfjjc/…
  webforge/models`). It is derived from `%LOCALAPPDATA%` (no user name) and is a
  *best-effort fallback*, not a contract — if that app moves its models, discovery
  simply misses and resolution falls through to an error, exactly as before.
- (−) **Not a downloader.** If no model is present anywhere, the app still drops to
  structural-only (and `llm_status` reports `model_present: false`). The
  first-run auto-download (alt #4) is the follow-up that closes that gap.
- (−) **The embedder still needs network on first run** to fetch its ONNX model
  into the temp-dir cache (ADR-0004); unchanged by this ADR, noted for the user.

## Invariant (pinned)

Resolution precedence and the directory scan are unit-tested in `doctree-llm`
(native-free, no model):

- **Precedence:** `resolve_with(env, discovered)` returns explicit `model_path`
  over `env` over `discovered`, ignores an empty env string, and **errors when all
  three are absent** (the old `resolve_model_path_errors_when_unset` guarantee,
  now deterministic and independent of the build machine's files).
- **Discovery:** `discover_model_in(dirs, preferred)` prefers the highest-priority
  preferred filename present, falls back to the first `.gguf` by sorted name,
  honours directory priority order, and returns `None` for empty/missing dirs.

Pinned test path: **`crates/doctree-llm/src/lib.rs`** `#[cfg(test)]`
(`resolve_prefers_explicit_then_env_then_discovery`,
`discover_model_in_prefers_known_names_then_first_gguf`,
`discover_model_in_honours_dir_priority_and_missing_dirs`). The standing gate
`cargo test --workspace` (no features) must stay green native-free.

## Anchors

- `crates/doctree-llm/src/lib.rs` — `PREFERRED_MODEL_FILES`, `discover_model_in`,
  `model_search_dirs`, `discover_model_path`, `LlmConfig::resolve_with` /
  `resolve_model_path`, and the pinned tests.
- `src-tauri/src/llm.rs` — `llm_status_impl` reports the resolved/discovered path
  via `LlmConfig::from_env().resolve_model_path().ok()`.
- `scripts/build-desktop.cmd` — release `--no-bundle --features llm,vectordb`
  (CPU) launcher mirroring `dev-desktop.cmd` (vcvars64 env).
