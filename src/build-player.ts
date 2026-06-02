import type { DocGraph, GraphNode, GraphEdge } from "./types";

// A single step in the animated build: a node appearing, or an edge wiring two
// already-present nodes. Mirrors `doctree_core::build::BuildStep` — the Rust
// walker stream (A8) emits exactly this tagged shape over Tauri events.
export type BuildEvent =
  | { kind: "node"; node: GraphNode }
  | { kind: "edge"; edge: GraphEdge };

const idOf = (ref: string | { id: string }): string =>
  typeof ref === "object" ? ref.id : ref;

// Deterministic build order — mirrors `doctree_core::build::build_sequence`:
// preserve the input node order (the walker emits its spine in document order,
// "from word one") and reveal each edge the instant both endpoints are present,
// flushing danglers last. Same graph in ⇒ same sequence out (faithful replay).
export function buildSequence(graph: DocGraph): BuildEvent[] {
  const present = new Set<string>();
  const emitted = new Set<number>();
  const seq: BuildEvent[] = [];

  for (const node of graph.nodes) {
    seq.push({ kind: "node", node });
    present.add(node.id);
    graph.edges.forEach((edge, i) => {
      if (
        !emitted.has(i) &&
        present.has(idOf(edge.source)) &&
        present.has(idOf(edge.target))
      ) {
        seq.push({ kind: "edge", edge });
        emitted.add(i);
      }
    });
  }
  // Edges whose endpoints never both appear (dangling refs) still get emitted so
  // the replayed graph is identical to the source graph.
  graph.edges.forEach((edge, i) => {
    if (!emitted.has(i)) seq.push({ kind: "edge", edge });
  });

  return seq;
}

export interface BuildPlayer {
  play(): void;
  pause(): void;
  toggle(): void;
  replay(): void;
  isPlaying(): boolean;
  isDone(): boolean;
}

export interface BuildPlayerOptions {
  sequence: BuildEvent[];
  intervalMs: number;
  // Push the current revealed subgraph into the view. Fresh arrays each call;
  // element refs are stable so already-placed nodes keep their positions and
  // only the newcomer animates in.
  apply: (nodes: GraphNode[], edges: GraphEdge[]) => void;
  onProgress?: (step: number, total: number, done: boolean) => void;
}

export function createBuildPlayer(opts: BuildPlayerOptions): BuildPlayer {
  const { sequence, intervalMs, apply, onProgress } = opts;
  const nodes: GraphNode[] = [];
  const edges: GraphEdge[] = [];
  let step = 0;
  let timer: number | null = null;

  const report = () =>
    onProgress?.(step, sequence.length, step >= sequence.length);

  function stop(): void {
    if (timer !== null) {
      window.clearInterval(timer);
      timer = null;
    }
  }

  function tick(): void {
    if (step >= sequence.length) {
      stop();
      return;
    }
    const ev = sequence[step++];
    if (ev.kind === "node") nodes.push(ev.node);
    else edges.push(ev.edge);
    apply([...nodes], [...edges]);
    // Stop before reporting on the final step so onProgress sees isPlaying()
    // false / done true (otherwise the last tick reports mid-play state).
    if (step >= sequence.length) stop();
    report();
  }

  function play(): void {
    if (timer !== null || step >= sequence.length) return;
    timer = window.setInterval(tick, intervalMs);
  }

  function pause(): void {
    stop();
    report();
  }

  function reset(): void {
    stop();
    step = 0;
    nodes.length = 0;
    edges.length = 0;
    apply([], []);
    report();
  }

  return {
    play,
    pause,
    toggle: () => (timer !== null ? pause() : play()),
    replay: () => {
      reset();
      play();
    },
    isPlaying: () => timer !== null,
    isDone: () => step >= sequence.length,
  };
}
