# ADR-0008 — GPU (CUDA) acceleration via the co-installed VS 2019 host compiler

- **Status:** Accepted (2026-06-02)
- **Phase:** 6 (#5)
- **Supersedes / superseded by:** none (closes the standing "GPU blocked" Catch-all item)

## Context

The desktop semantic / embedding pass is CPU-bound and slow on a large document
(the user's 650 KB novel → 18 437 nodes / 46 477 edges takes tens of seconds).
The machine has an **RTX 3060 Ti (8 GB)** sitting idle, and the model in use
(`qwen3-4b-q4km.gguf`, ~2.33 GB) fits comfortably in VRAM — so GPU offload is a
free, large win if it can be built.

Earlier phases recorded GPU as **blocked** (B1, BUILD_LOG Catch-all): the gated
`cuda` / `vulkan` sub-features "do NOT build on this machine." Re-probing on
2026-06-02 showed **that note was stale about the cause.** The block was never
"CUDA can't compile our code." It was **nvcc refusing the host C++ compiler**:
`nvcc` (CUDA **13.1**, `V13.1.115`) only supports **VS 2019–2022** host
toolchains and rejects the **VS 2026** toolset (MSVC **14.50**) that the standard
`scripts/dev-desktop.cmd` loads via its `vcvars64`. `llama.cpp`'s CUDA backend
never even reached its CMake try-compile.

Probing further found a **VS 2019 BuildTools install already present** at
`C:\Program Files (x86)\Microsoft Visual Studio\2019\BuildTools` with **MSVC
14.29.30133** (`cl.exe`), bundled CMake + Ninja, and a Windows SDK — i.e. a
**CUDA-13.1-accepted host compiler is already on disk.** And the feature wiring
already cascades end-to-end:

```
doctree-tauri  --features cuda
  └─ doctree-llm/cuda  =  ["llm", "momusdev_llm/cuda"]
       └─ momusdev_llm/cuda  →  llama-cpp-2/cuda  →  llama.cpp CUDA backend
```

So GPU unblocks with **no install** and **no change to the read-only sibling
crates** — only the build *environment* has to change.

## Decision

**Enable CUDA, built from the co-installed VS 2019 BuildTools environment.**

- `cuda` stays a **Cargo feature flag**. The default build remains **native-free**
  (ADR-0001) and **CPU remains the guaranteed fallback** — `cuda` is strictly
  opt-in.
- The GPU build runs from the **VS 2019** developer environment (so `cl` on PATH
  is MSVC 14.29, which nvcc accepts and CMake detects), instead of the VS 2026
  environment the CPU launcher uses.
- Ship `scripts/dev-desktop-gpu.cmd`: a sibling of `dev-desktop.cmd` that calls
  the **VS 2019** `vcvars64.bat` (overridable via `DOCTREE_VS2019_VCVARS`) and
  runs `npm run tauri -- dev --features llm,cuda`.
- **GPU scope is `llm,cuda` — NOT `vectordb`.** CUDA accelerates the slow part
  (the llama.cpp LLM/semantic pass); embeddings/vector-store stays CPU. This is
  forced, not preferred: see "vectordb + cuda cannot co-link here" below.
- If a future CUDA build hits an unforeseen wall, the decision **degrades
  gracefully to CPU-only** (documented) because `cuda` is opt-in and CPU is the
  default path.

### vectordb + cuda cannot co-link on this machine (empirical, 2026-06-02)

The first GPU launcher targeted `--features llm,vectordb,cuda` (the desktop's full
capability set). That **compiles but fails at link** under VS 2019: 39 unresolved
external symbols, all newer-MSVC STL internals (`__std_max_element_1`,
`__std_minmax_element_4`, the charconv tables `__DOUBLE_POW5_INV_SPLIT` /
`_General_precision_tables_2`, …) pulled in by **`libort_sys`** — the prebuilt
ONNX Runtime that `vectordb` brings in (via `fastembed` → `ort`). That prebuilt
binary references STL symbols that **MSVC 14.29 (VS 2019) does not provide**; VS
2026's MSVC 14.50 *has* them, but nvcc rejects VS 2026 as a host compiler. So the
two features want **opposite, mutually exclusive** toolchains:

- `cuda` → needs **VS 2019–2022** (nvcc host-compiler rule).
- `vectordb`'s prebuilt ORT → needs **VS 2022+ STL** symbols VS 2019 lacks.

VS 2019 satisfies nvcc but not ORT; VS 2026 satisfies ORT but not nvcc. They
cannot both be satisfied on this machine today. **Resolution:** the GPU build
drops `vectordb`. The `cuda` and `vectordb` Cargo features are already independent
(`cuda` does not pull `vectordb`), so `--features llm,cuda` is a clean, valid
build — and `vectordb` continues to build CPU-side under the normal VS 2026
launcher. The future unifier is **a single VS 2022 toolset** (MSVC 14.3x), which
nvcc accepts *and* whose STL provides ORT's symbols; installing it would let
`llm,vectordb,cuda` link in one environment. That's a tooling upgrade, not a code
change, so it's left as a documented future path rather than a blocker.

**Validation (this ADR's pinned evidence).** Built from the VS 2019 env, two
levels:

1. **Crate-level (pinned invariant):** `cargo build -p doctree-llm --features cuda`
   → the long llama.cpp CUDA native compile (nvcc over the CUDA kernels) ran clean
   and then cached; a re-run reports `Finished dev profile … in 0.40s`, **cargo
   exit code 0**, with `cl` resolving to
   `…\2019\BuildTools\VC\Tools\MSVC\14.29.30133\bin\Hostx64\x64\cl.exe`.
2. **Full-app link (the corrected GPU scope):** `cargo build -p doctree-tauri
   --features llm,cuda` → **`Finished dev profile … in 11m 49s`, cargo exit code
   0.** The whole desktop binary links clean with CUDA when `vectordb` is excluded
   — confirming the scope chosen above. (The same build *with* `vectordb` added is
   the one that fails at link with the 39 ORT STL externals.)

The only warnings are pre-existing sibling-crate lint noise
(`momusdev_met`/`momusdev_llm`), not errors. The **live GPU offload** (model
layers reported on `dev = CUDA`, faster extraction wall-time) is the user's
desktop end test — the agent proves **compile + link**; the running webview isn't
headlessly introspectable (the documented A5–D4 limit).

## Alternatives considered

1. **`-allow-unsupported-compiler` to force the VS 2026 host.** **Rejected:** the
   Catch-all already recorded that this flag doesn't get the backend through
   CMake's try-compile; forcing an unsupported MSVC is also fragile across nvcc
   updates. Using an actually-supported compiler is the correct fix.
2. **Vulkan instead of CUDA.** The Vulkan SDK (1.4.341.1) is on PATH, but
   `vulkan-shaders-gen` failed under MSVC 14.50, and CUDA is the better fit for an
   NVIDIA RTX 3060 Ti anyway. **Rejected** for this machine.
3. **Install a fresh VS 2019–2022 toolset.** **Rejected:** unnecessary — a
   compatible toolset (VS 2019 BuildTools) already exists on disk.
4. **Patch the sibling crates' build to select the toolchain.** **Forbidden:**
   `momusdev_llm` / `momusdev_met` are read-only. The host-compiler choice is an
   environment concern (which `vcvars64` is active), solved entirely in our
   launcher — no crate edit needed.
5. **Stay CPU-only.** **Rejected:** leaves the GPU idle and extraction slow when a
   free win is available with zero install and zero crate change.

## Consequences

- (+) The semantic/embedding pass can offload to the GPU, cutting extraction
  wall-time on large documents; the 3060 Ti is no longer idle.
- (+) Zero new installs, **zero sibling-crate changes** — only the build
  environment differs (VS 2019 vs VS 2026).
- (+) The default/public build is untouched and still native-free (ADR-0001);
  CPU is the guaranteed fallback.
- (−) GPU requires running the dedicated launcher from the VS 2019 environment;
  building `cuda` from the VS 2026 env still fails (by nvcc's rule, not ours) —
  documented in both launchers.
- (−) **GPU and `vectordb` can't co-build here:** the GPU launcher runs
  `llm,cuda` only, so on the GPU path embeddings/RAG fall back to CPU (or are run
  separately under the VS 2026 CPU launcher). Unifying them needs a VS 2022
  toolset (future path), not a code change.
- (−) The first `cuda` build is a long llama.cpp CUDA compile (minutes); cached
  thereafter.
- (−) Two launchers to keep in sync (CPU vs GPU) — acceptable; they differ only
  in the `vcvars64` path and the feature set (`…,vectordb` CPU vs `llm,cuda` GPU).

## Invariant (pinned)

- **Validation:** `cargo build -p doctree-llm --features cuda` compiles and links
  to `Finished` (exit 0) **when run from the VS 2019 `vcvars64` environment**
  (MSVC 14.29); and the full app `cargo build -p doctree-tauri --features llm,cuda`
  likewise links to `Finished` (exit 0). This is the agent-side proof; the live
  GPU-offload run is the user's desktop end test. **`vectordb` must NOT be added to
  the GPU feature set** — `--features llm,vectordb,cuda` fails at link under VS
  2019 (ORT prebuilt vs MSVC 14.29 STL); GPU scope is `llm,cuda`.
- **Gate (unchanged default):** `cargo test --workspace` (no features) stays
  green with **zero** native deps — `cuda` is opt-in and must never enter the
  default build (ADR-0001).
- **Read-only siblings:** enabling CUDA required **no** edit to `momusdev_llm` /
  `momusdev_met`; the `cuda` feature consumes their published cascade as-is. If a
  future CUDA need can't be met by the published API, flag it under Catch-all —
  do not patch the crate.
- **Toolchain fact:** the CUDA-accepted host compiler is VS 2019 BuildTools
  (MSVC 14.29.30133) at `C:\Program Files (x86)\Microsoft Visual Studio\2019\
  BuildTools`; nvcc is CUDA 13.1. VS 2026 (MSVC 14.50) is **not** nvcc-accepted.
