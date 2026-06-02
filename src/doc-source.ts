import type { BuildEvent } from "./build-player";
import { buildSequence } from "./build-player";
import type { DocGraph, NodeKind } from "./types";
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
  // B5 frontend wiring: the classification verdict + routing (desktop only —
  // `classify_document` is native-free and registered on every build, but only
  // the Tauri shell exposes it; the browser walks structurally via WASM).
  routing?: Routing;
  // The command actually invoked. Equals `routing.command` on the happy path, or
  // "build_steps" when the resolved semantic/embedded command failed at runtime
  // (e.g. the feature is compiled but no model file is on disk) and we fell back.
  command?: string;
  // True when the resolved command failed and we degraded to the structural spine.
  fellBack?: boolean;
}

// --- B5 routing DTOs (mirror src-tauri/src/lib.rs `Routing` + core classify) ---
// Serde emits camelCase fields with snake_case enum *values* (the schema tags).
export type DocumentClass = "narrative" | "expository" | "structured" | "unknown";
export type RecommendedPipeline =
  | "narrative_hybrid"
  | "structural_plus_similarity"
  | "structural_only";
export type ResolvedPipeline = "semantic_build" | "embedded_build" | "structural_build";

export interface ClassificationSignals {
  wordCount: number;
  structureRatio: number;
  dialogueRatio: number;
  pronounRatio: number;
  pastTenseRatio: number;
}

export interface Capabilities {
  llm: boolean;
  vectordb: boolean;
}

export interface Routing {
  class: DocumentClass;
  confidence: number;
  signals: ClassificationSignals;
  recommendedPipeline: RecommendedPipeline;
  resolvedPipeline: ResolvedPipeline;
  /** The registered Tauri command to invoke for this document. */
  command: string;
  /** The ideal pipeline for this class isn't compiled into this build. */
  downgraded: boolean;
  capabilities: Capabilities;
}

// One semantic-search hit (mirrors `SearchHitDto` in src-tauri/src/llm.rs).
export interface MeaningHit {
  id: string;
  label: string;
  kind: NodeKind;
  score: number;
}

const fixture = fixtureJson as unknown as DocGraph;

// Tauri v2 injects `__TAURI_INTERNALS__` into the webview; its absence means we
// are in a normal browser (public web build) and walk via WASM instead.
export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

// Cached handle to Tauri's `invoke`. The dynamic import keeps the desktop-only
// API out of the browser bundle's critical path.
let invokeReady: Promise<typeof import("@tauri-apps/api/core").invoke> | null = null;
async function tauriInvoke<T>(cmd: string, args: Record<string, unknown>): Promise<T> {
  if (!invokeReady) {
    invokeReady = import("@tauri-apps/api/core").then((m) => m.invoke);
  }
  const invoke = await invokeReady;
  return invoke<T>(cmd, args);
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

// Desktop (Tauri) path: classify the document (B5), dispatch to the *resolved*
// build command the classifier names — `semantic_build_steps` (LLM hybrid),
// `embedded_build_steps` (similarity), or `build_steps` (spine) — and gracefully
// fall back to the structural spine if the richer command fails at runtime (the
// feature is compiled but, say, the model file is missing). The classifier itself
// is native-free, so the *routing* always succeeds even on a lean build; only the
// model-backed build commands can fail, and those degrade to `build_steps`.
async function loadTauriBuildSource(text: string): Promise<BuildSource> {
  const routing = await tauriInvoke<Routing>("classify_document", { text });
  const resolved = routing.command;
  try {
    const steps = await tauriInvoke<BuildEvent[]>(resolved, { text });
    return {
      sequence: steps,
      origin: "tauri-walk",
      ...tally(steps),
      routing,
      command: resolved,
      fellBack: false,
    };
  } catch (err) {
    if (resolved === "build_steps") throw err; // the spine is the floor — nothing below it
    console.warn(`${resolved} failed; falling back to structural build_steps:`, err);
    const steps = await tauriInvoke<BuildEvent[]>("build_steps", { text });
    return {
      sequence: steps,
      origin: "tauri-walk",
      ...tally(steps),
      routing,
      command: "build_steps",
      fellBack: true,
    };
  }
}

// Resolve the build sequence for `text` using the best available walk engine.
// Desktop → classify + resolved semantic/structural command; browser → WASM
// (structural only — no model in the browser); fixture only as a hard fallback so
// the page never renders nothing.
export async function loadBuildSource(
  text: string = defaultDocument
): Promise<BuildSource> {
  if (isTauri()) {
    return loadTauriBuildSource(text);
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

// "Find by meaning" (B4): rank the document's nodes by embedding similarity to a
// free-text query. Desktop + `vectordb` only — returns [] anywhere else (the
// caller gates the UI on `routing.capabilities.vectordb`, so this is a defensive
// floor). The backend re-walks `text` deterministically, so the returned ids line
// up with the nodes already on screen.
export async function searchByMeaning(
  query: string,
  text: string,
  topK = 10
): Promise<MeaningHit[]> {
  if (!isTauri()) return [];
  return tauriInvoke<MeaningHit[]>("semantic_search", { query, text, topK });
}

export { fixture };
