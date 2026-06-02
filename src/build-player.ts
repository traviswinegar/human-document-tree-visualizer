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
  // Re-pace the animation. Takes effect immediately, even mid-build.
  setSpeed(intervalMs: number): void;
  isPlaying(): boolean;
  isDone(): boolean;
}

export interface BuildPlayerOptions {
  sequence: BuildEvent[];
  // Target pace: milliseconds *per step*. This is a rate, not a timer period —
  // the player ticks at a fixed frame cadence and reveals however many steps are
  // due by elapsed wall-clock, so small values fold many steps into one frame.
  intervalMs: number;
  // Push the current revealed subgraph into the view. Fresh arrays each call;
  // element refs are stable so already-placed nodes keep their positions and
  // only the newcomers animate in.
  apply: (nodes: GraphNode[], edges: GraphEdge[]) => void;
  onProgress?: (step: number, total: number, done: boolean) => void;
}

// The timer fires at roughly one animation frame; each fire reveals all steps
// that have come *due* since the pacing anchor. Throughput is therefore bounded
// by one graphData() apply per frame, not by the per-step render cost — so the
// fast end of the speed range can fold dozens of steps into a single repaint and
// approach an instant build, while the slow end still drips one step at a time.
const FRAME_MS = 16;

export function createBuildPlayer(opts: BuildPlayerOptions): BuildPlayer {
  const { sequence, apply, onProgress } = opts;
  // Mutable so the speed slider can re-pace a build already in flight.
  let intervalMs = opts.intervalMs;
  const nodes: GraphNode[] = [];
  const edges: GraphEdge[] = [];
  let step = 0;
  let timer: number | null = null;
  // Pacing anchor: steps due = anchorStep + floor((now - anchorTime) / intervalMs).
  // Re-anchored on play()/setSpeed() so a pace change starts from "now" and never
  // retroactively jumps (or rewinds) the build.
  let anchorTime = 0;
  let anchorStep = 0;

  const report = () =>
    onProgress?.(step, sequence.length, step >= sequence.length);

  function stop(): void {
    if (timer !== null) {
      window.clearInterval(timer);
      timer = null;
    }
  }

  function tick(): void {
    const elapsed = performance.now() - anchorTime;
    const due = anchorStep + Math.floor(elapsed / intervalMs);
    const target = Math.min(due, sequence.length);
    if (target > step) {
      // Drain every due step into the working arrays, then apply once. Batching
      // the graphData() call is the whole point — N steps, one repaint.
      for (; step < target; step++) {
        const ev = sequence[step];
        if (ev.kind === "node") nodes.push(ev.node);
        else edges.push(ev.edge);
      }
      apply([...nodes], [...edges]);
    }
    // Stop before the final report so onProgress sees isPlaying() false / done
    // true (otherwise the completing tick would report a mid-play state).
    if (step >= sequence.length) stop();
    report();
  }

  function play(): void {
    if (timer !== null || step >= sequence.length) return;
    anchorTime = performance.now();
    anchorStep = step;
    timer = window.setInterval(tick, FRAME_MS);
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

  // Change the per-step pace. Re-anchor to the current step/time so the new rate
  // takes effect immediately and smoothly, with no retroactive jump.
  function setSpeed(ms: number): void {
    intervalMs = ms;
    anchorTime = performance.now();
    anchorStep = step;
  }

  return {
    play,
    pause,
    toggle: () => (timer !== null ? pause() : play()),
    replay: () => {
      reset();
      play();
    },
    setSpeed,
    isPlaying: () => timer !== null,
    isDone: () => step >= sequence.length,
  };
}
