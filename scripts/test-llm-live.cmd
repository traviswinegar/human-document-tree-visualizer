@echo off
setlocal enableextensions
REM ---------------------------------------------------------------------------
REM test-llm-live.cmd - run the #[ignore]d live LLM tests against a real GGUF.
REM
REM The default `cargo test` is native-free and never touches a model. The
REM grammar-constrained extraction path can only be exercised end to end with a
REM model loaded, so those tests are #[ignore]d. This runner mirrors
REM dev-desktop.cmd's environment setup (MSVC build env + DOCTREE_MODEL_PATH) and
REM runs ONLY the ignored tests in doctree-llm, with output captured.
REM
REM Use it to verify the semantic layer without launching the whole desktop app
REM (the Tauri webview is not introspectable from the build harness).
REM ---------------------------------------------------------------------------

call "E:\Program Files\Visual Studio\VC\Auxiliary\Build\vcvars64.bat"
if errorlevel 1 (
  echo [test-llm-live] FAILED to load the MSVC build environment ^(vcvars64^).
  exit /b 1
)

if not defined DOCTREE_MODEL_PATH (
  set "DOCTREE_MODEL_PATH=C:\Users\travi\AppData\Local\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Roaming\com.example\webforge\webforge\models\qwen3-4b-q4km.gguf"
)

if not exist "%DOCTREE_MODEL_PATH%" (
  echo [test-llm-live] model not found at:
  echo                 %DOCTREE_MODEL_PATH%
  echo                 Set DOCTREE_MODEL_PATH to a .gguf and re-run.
  exit /b 1
)

echo [test-llm-live] model: %DOCTREE_MODEL_PATH%
echo [test-llm-live] running ignored live tests in doctree-llm (--features llm)
cargo test -p doctree-llm --features llm -- --ignored --nocapture
