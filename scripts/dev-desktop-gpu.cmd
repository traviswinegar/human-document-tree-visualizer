@echo off
setlocal enableextensions
REM ---------------------------------------------------------------------------
REM dev-desktop-gpu.cmd - launch the FULL desktop app with GPU (CUDA) offload.
REM
REM Same as dev-desktop.cmd (structural spine + semantic LLM/embedding layer),
REM but compiles the gated llama.cpp backend with CUDA so the model offloads its
REM layers to the GPU (the user's RTX 3060 Ti) instead of running CPU-only. See
REM ADR-0008. The default build stays native-free (ADR-0001); CUDA is opt-in via
REM the `cuda` feature and CPU remains the guaranteed fallback.
REM
REM WHY A SEPARATE LAUNCHER (and a different MSVC):
REM   nvcc (CUDA 13.1) only accepts VS 2019-2022 host compilers and REFUSES the
REM   VS 2026 toolchain (MSVC 14.50) that dev-desktop.cmd loads. The block was
REM   never our code - it was the host-compiler version. A VS 2019 BuildTools
REM   install (MSVC 14.29.30133) is already on this machine and nvcc accepts it,
REM   so this launcher loads THAT environment (its `cl` lands on PATH and CMake
REM   detects it) before building. No install, no sibling-crate change - the
REM   `cuda` feature already cascades doctree-llm -> momusdev_llm -> llama-cpp-2.
REM
REM WHY NO `vectordb` HERE (features = llm,cuda, not llm,vectordb,cuda):
REM   vectordb pulls a PREBUILT ONNX Runtime (fastembed -> ort -> libort_sys) that
REM   references newer-MSVC STL symbols VS 2019 (14.29) does not provide, so adding
REM   it makes the whole app FAIL at link under VS 2019 (39 unresolved ORT STL
REM   externals). cuda needs VS 2019-2022; ORT needs VS 2022+ STL - mutually
REM   exclusive on this machine. So GPU scope is llm,cuda (CUDA accelerates the
REM   slow LLM pass); embeddings/RAG run CPU-side under dev-desktop.cmd. A single
REM   VS 2022 toolset would satisfy both - future path, not a code change. ADR-0008.
REM ---------------------------------------------------------------------------

REM VS 2019 BuildTools - the CUDA-13.1-accepted host compiler. Override
REM DOCTREE_VS2019_VCVARS if your VS 2019 lives elsewhere.
if not defined DOCTREE_VS2019_VCVARS (
  set "DOCTREE_VS2019_VCVARS=C:\Program Files (x86)\Microsoft Visual Studio\2019\BuildTools\VC\Auxiliary\Build\vcvars64.bat"
)
call "%DOCTREE_VS2019_VCVARS%"
if errorlevel 1 (
  echo [dev-desktop-gpu] FAILED to load the VS 2019 MSVC build environment ^(vcvars64^).
  echo                   CUDA needs a VS 2019-2022 host compiler; set
  echo                   DOCTREE_VS2019_VCVARS to your vcvars64.bat and retry,
  echo                   or use scripts\dev-desktop.cmd for the CPU build.
  exit /b 1
)

if not defined DOCTREE_MODEL_PATH (
  set "DOCTREE_MODEL_PATH=C:\Users\travi\AppData\Local\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Roaming\com.example\webforge\webforge\models\qwen3-4b-q4km.gguf"
)

if not exist "%DOCTREE_MODEL_PATH%" (
  echo [dev-desktop-gpu] WARNING: model not found at:
  echo                   %DOCTREE_MODEL_PATH%
  echo                   The semantic layer will fall back to the structural spine.
  echo                   Set DOCTREE_MODEL_PATH to a .gguf to enable LLM extraction.
)

echo [dev-desktop-gpu] cl (host compiler nvcc will use):
where cl
echo [dev-desktop-gpu] model: %DOCTREE_MODEL_PATH%
echo [dev-desktop-gpu] launching: tauri dev --features llm,cuda
echo [dev-desktop-gpu] (no vectordb: ORT prebuilt won't link under VS 2019; see ADR-0008)
echo [dev-desktop-gpu] (first CUDA build is a long llama.cpp compile; cached after.)
call npm run tauri -- dev --features llm,cuda
