import type { BuildEvent } from "./build-player";
import { buildSequence } from "./build-player";
import type { DocGraph } from "./types";
import fixtureJson from "../fixtures/sample-narrative.graph.json";
import sampleText from "../fixtures/sample-narrative.txt?raw";

// Where the animated build's steps came from: the live Rust walker over the
// Tauri bridge, or — when running in a plain browser (no Tauri shell) — the
// pre-baked fixture graph. Same `BuildEvent[]` shape either way, so the player
// and view code (src/main.ts) are agnostic to the origin.
export interface BuildSource {
  sequence: BuildEvent[];
  origin: "tauri-walk" | "fixture";
  nodeCount: number;
  edgeCount: number;
}

const fixture = fixtureJson as unknown as DocGraph;

// Tauri v2 injects `__TAURI_INTERNALS__` into the webview; its absence means we
// are in a normal browser (e.g. `vite dev` opened directly) and must fall back.
export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

// The document Tauri mode walks by default. Phase 1 has no file-open dialog yet,
// so the round-trip is proven against the same narrative the fixture was hand-
// authored from — but the graph here is produced live by the Rust walker, not
// the baked fixture, which is the whole point of the A8 bridge.
export const defaultDocument = sampleText;

// Resolve the build sequence for `text`. In the Tauri shell this calls the Rust
// `build_steps` command (doc → deterministic walker → ordered steps); the
// returned `BuildStep` JSON is exactly our `BuildEvent` tagged shape, so it
// flows straight into the player. In a browser it replays the fixture.
export async function loadBuildSource(
  text: string = defaultDocument
): Promise<BuildSource> {
  if (isTauri()) {
    const { invoke } = await import("@tauri-apps/api/core");
    const steps = await invoke<BuildEvent[]>("build_steps", { text });
    return {
      sequence: steps,
      origin: "tauri-walk",
      nodeCount: steps.filter((s) => s.kind === "node").length,
      edgeCount: steps.filter((s) => s.kind === "edge").length,
    };
  }
  return {
    sequence: buildSequence(fixture),
    origin: "fixture",
    nodeCount: fixture.nodes.length,
    edgeCount: fixture.edges.length,
  };
}

export { fixture };
