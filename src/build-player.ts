import type { DocGraph, GraphNode, GraphEdge, NodeKind } from "./types";

// A single step in the animated build: a node appearing, or an edge wiring two
// already-present nodes. The walker (A8) will eventually emit these in real
// discovery order; until then we synthesize a deterministic order from a graph.
export type BuildEvent =
  | { kind: "node"; node: GraphNode }
  | { kind: "edge"; edge: GraphEdge };

// Structure grows before meaning: the deterministic spine (section→…→term) lays
// down first, then the LLM-style semantic overlay (characters, places, …) lights
// up on top. Lower rank = earlier in the build.
const KIND_RANK: Record<NodeKind, number> = {
  section: 0,
  paragraph: 1,
  sentence: 2,
  clause: 3,
  quote: 4,
  reference: 5,
  term: 6,
  character: 7,
  place: 8,
  object: 9,
  event: 10,
  concept: 11,
  group: 12,
};

const idOf = (ref: string | { id: string }): string =>
  typeof ref === "object" ? ref.id : ref;

// Deterministic build order: nodes by (kind rank, id); each edge is emitted the
// moment both its endpoints are present, in declaration order. Same input →
// same sequence (this is what makes replay faithful).
export function buildSequence(graph: DocGraph): BuildEvent[] {
  const nodes = [...graph.nodes].sort((a, b) => {
    const r = KIND_RANK[a.kind] - KIND_RANK[b.kind];
    return r !== 0 ? r : a.id.localeCompare(b.id);
  });

  const present = new Set<string>();
  const emitted = new Set<number>();
  const seq: BuildEvent[] = [];

  for (const node of nodes) {
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
