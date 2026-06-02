@echo off
setlocal enableextensions
REM ---------------------------------------------------------------------------
REM build-desktop.cmd - build the STANDALONE, FULL-capability release .exe
REM (structural spine + semantic LLM + embeddings) as a single double-clickable
REM binary with NO env setup at runtime (ADR-00014).
REM
REM Why release (not the debug binary): a debug `tauri dev` build loads its UI
REM from the Vite dev server (build.devUrl) and shows a BLANK window when double-
REM clicked with no server running. `tauri build` runs beforeBuildCommand
REM (`npm run build` -> dist/) and EMBEDS the assets, so the exe is self-running.
REM `--no-bundle` skips installers and emits just the standalone exe.
REM
REM Why CPU (llm,vectordb) and NOT cuda: a CUDA build needs VS 2019 + CUDA
REM runtime DLLs travelling beside the exe (ADR-0008) = the "special whatevers"
REM the single-exe ask rules out. CPU inference is fully self-contained. Power
REM users wanting GPU offload still have scripts\dev-desktop-gpu.cmd.
REM
REM The model is found at RUNTIME with zero config via doctree_llm's
REM discover_model_path (ADR-00014): a .gguf next to the exe / in <exe>\models\,
REM the app's LocalAppData models dir, or the known webforge model cache. No
REM DOCTREE_MODEL_PATH needed (it still works as an override if you set it).
REM
REM   1. Loads the MSVC build environment (to compile the gated llama.cpp backend).
REM   2. Runs `tauri build --no-bundle` with the gated features on.
REM
REM Output: target\release\doctree-tauri.exe
REM ---------------------------------------------------------------------------

call "E:\Program Files\Visual Studio\VC\Auxiliary\Build\vcvars64.bat"
if errorlevel 1 (
  echo [build-desktop] FAILED to load the MSVC build environment ^(vcvars64^).
  exit /b 1
)

echo [build-desktop] building: tauri build --no-bundle --features llm,vectordb (CPU)
echo [build-desktop] this is a release llama.cpp compile - expect several minutes.
call npm run tauri -- build --no-bundle --features llm,vectordb
if errorlevel 1 (
  echo [build-desktop] BUILD FAILED.
  exit /b 1
)

echo.
echo [build-desktop] DONE. Standalone exe:
echo                 target\release\doctree-tauri.exe
echo                 Double-click it - no PATH, no env, no install required.
echo                 (First run downloads the embedding model once; needs network.)
