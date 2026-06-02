@echo off
setlocal enableextensions
REM ---------------------------------------------------------------------------
REM dev-desktop.cmd - launch the FULL desktop app (structural spine + semantic
REM LLM/embedding layer).
REM
REM The browser / public-web build walks via WASM and is STRUCTURAL-ONLY by
REM design: a browser tab can't load a multi-GB local model, and the core
REM pipeline makes no cloud calls. The semantic overlay (character/place/concept/
REM event nodes from the grammar-constrained LLM, plus similarity edges and
REM "find by meaning") only exists on the desktop. This launcher is how you run
REM that full product.
REM
REM   1. Loads the MSVC build environment (needed to compile the gated
REM      llama.cpp backend behind the `llm` feature).
REM   2. Points DOCTREE_MODEL_PATH at a local GGUF model. Override it by setting
REM      the env var before running this; otherwise it defaults to the qwen3-4b
REM      recorded in BUILD_LOG's Phase 0 facts.
REM   3. Runs `tauri dev` with the gated features on. CPU inference only - GPU
REM      (cuda/vulkan) is blocked on this toolchain; see BUILD_LOG Catch-all.
REM ---------------------------------------------------------------------------

call "E:\Program Files\Visual Studio\VC\Auxiliary\Build\vcvars64.bat"
if errorlevel 1 (
  echo [dev-desktop] FAILED to load the MSVC build environment ^(vcvars64^).
  exit /b 1
)

if not defined DOCTREE_MODEL_PATH (
  set "DOCTREE_MODEL_PATH=C:\Users\travi\AppData\Local\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Roaming\com.example\webforge\webforge\models\qwen3-4b-q4km.gguf"
)

if not exist "%DOCTREE_MODEL_PATH%" (
  echo [dev-desktop] WARNING: model not found at:
  echo                %DOCTREE_MODEL_PATH%
  echo                The semantic layer will fall back to the structural spine.
  echo                Set DOCTREE_MODEL_PATH to a .gguf to enable LLM extraction.
)

echo [dev-desktop] model: %DOCTREE_MODEL_PATH%
echo [dev-desktop] launching: tauri dev --features llm,vectordb
call npm run tauri -- dev --features llm,vectordb
