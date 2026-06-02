import type { BuildEvent } from "./build-player";
import { buildSequence } from "./build-player";
import type { DocGraph } from "./types";
import fixtureJson from "../fixtures/sample-narrative.graph.json";
import sampleText from "../fixtures/sample-narrative.txt?raw";

// Where the animated build's steps came from. All three run the *same*
// authoritative spine ordering; they differ only in which engine walked the
// document:
//   - "tauri-walk": the native Rust walker over Tauri IPC (desktop shell, A8);
//   - "wasm-walk":  the same Rust walker compiled to WebAssembly (browser, ADR-0003);
//   - "fixture":    the baked sample graph, only if the wasm module fails to load.
// Same `BuildEvent[]` shape either way, so the player/view code is engine-agnostic.
export interface BuildSource {
  sequence: BuildEvent[];
  origin: "tauri-walk" | "wasm-walk" | "fixture";
  nodeCount: number;
  edgeCount: number;
}

const fixture = fixtureJson as unknown as DocGraph;

// Tauri v2 injects `__TAURI_INTERNALS__` into the webview; its absence means we
// are in a normal browser (public web build) and walk via WASM instead.
export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

// The document walked on first load (before the user uploads their own). Phase 1
// ships one bundled narrative so the page renders a real walk immediately.
export const defaultDocument = sampleText;

// Lazily instantiate the WASM walker once and reuse it. The dynamic import keeps
// the ~180 KB wasm glue out of the initial bundle until a browser walk is needed.
let wasmReady: Promise<typeof import("./wasm/doctree_wasm.js")> | null = null;
async function getWasm(): Promise<typeof import("./wasm/doctree_wasm.js")> {
  if (!wasmReady) {
    wasmReady = import("./wasm/doctree_wasm.js").then(async (mod) => {
      await mod.default(); // instantiate the .wasm
      return mod;
    });
  }
  return wasmReady;
}

const tally = (steps: BuildEvent[]) => ({
  nodeCount: steps.filter((s) => s.kind === "node").length,
  edgeCount: steps.filter((s) => s.kind === "edge").length,
});

// Resolve the build sequence for `text` using the best available walk engine.
// Desktop → native command; browser → WASM; fixture only as a hard fallback so
// the page never renders nothing.
export async function loadBuildSource(
  text: string = defaultDocument
): Promise<BuildSource> {
  if (isTauri()) {
    const { invoke } = await import("@tauri-apps/api/core");
    const steps = await invoke<BuildEvent[]>("build_steps", { text });
    return { sequence: steps, origin: "tauri-walk", ...tally(steps) };
  }
  try {
    const wasm = await getWasm();
    // buildSteps returns the same tagged JSON string the Tauri command produces.
    const steps = JSON.parse(wasm.buildSteps(text)) as BuildEvent[];
    return { sequence: steps, origin: "wasm-walk", ...tally(steps) };
  } catch (err) {
    console.error("WASM walk failed; falling back to fixture:", err);
    return {
      sequence: buildSequence(fixture),
      origin: "fixture",
      nodeCount: fixture.nodes.length,
      edgeCount: fixture.edges.length,
    };
  }
}

export { fixture };
