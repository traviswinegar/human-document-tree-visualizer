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
//
// An edge becomes revealable exactly when the *later* of its two endpoints has
// appeared, i.e. at node index max(srcIndex, tgtIndex). Bucketing edges by that
// index (in their original array order within each bucket) reproduces the
// reveal-as-soon-as-both-present ordering in O(n + e) instead of the naïve
// O(n · e) rescan — which matters when a restored graph carries tens of thousands
// of edges (the 650 KB novel is ~46k) and the sequence is rebuilt to replay it.
export function buildSequence(graph: DocGraph): BuildEvent[] {
  // First-appearance index of each node id (later duplicates, if any, defer to the
  // first, matching the original "present once added" semantics).
  const indexOf = new Map<string, number>();
  graph.nodes.forEach((node, i) => {
    if (!indexOf.has(node.id)) indexOf.set(node.id, i);
  });

  // readyAfter[i] = edges to emit immediately after node i is revealed.
  const readyAfter: BuildEvent[][] = graph.nodes.map(() => []);
  const danglers: BuildEvent[] = []; // an endpoint that never appears as a node
  for (const edge of graph.edges) {
    const s = indexOf.get(idOf(edge.source));
    const t = indexOf.get(idOf(edge.target));
    if (s === undefined || t === undefined) danglers.push({ kind: "edge", edge });
    else readyAfter[Math.max(s, t)].push({ kind: "edge", edge });
  }

  const seq: BuildEvent[] = [];
  graph.nodes.forEach((node, i) => {
    seq.push({ kind: "node", node });
    for (const ev of readyAfter[i]) seq.push(ev);
  });
  // Dangling edges still get emitted last so the replayed graph is identical to
  // the source graph.
  for (const ev of danglers) seq.push(ev);

  return seq;
}

export interface BuildPlayer {
  play(): void;
  pause(): void;
  toggle(): void;
  replay(): void;
  // Re-pace the animation. Takes effect immediately, even mid-build.
  setSpeed(intervalMs: number): void;
  // Weave more steps onto the end of an in-flight (or already-finished) build.
  // Used by the desktop hybrid path: the structural spine streams first, then the
  // slow model-backed semantic/embedding delta is appended the moment it arrives,
  // so the graph keeps growing instead of the user staring at a static spine.
  append(events: BuildEvent[]): void;
  // Jump straight to the finished state *without animating or repainting* — used
  // when a saved graph is restored whole (it lands on its stored layout at once).
  // The caller has already populated the scene, so this only syncs internal state
  // (step + working arrays) so isDone() reports true and a later replay() can
  // re-stream the whole growth from empty. Deliberately does not call apply or
  // onProgress (the caller owns the static presentation).
  complete(): void;
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
  const { apply, onProgress } = opts;
  // A private, growable copy of the steps: append() extends it without mutating
  // the caller's source array (main.ts also hands that same array to the dev
  // handle), and replay() then re-runs the whole grown sequence (spine + delta).
  const sequence = [...opts.sequence];
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

  // Extend the build with late-arriving steps (the semantic/embedding delta on the
  // desktop hybrid path). Re-anchor to "now" so the new tail animates at the
  // current pace instead of dumping in one frame — the spine usually finishes long
  // before the model does — and restart the frame timer if the build had already
  // completed. A paused build stays paused (the user is in control); the appended
  // steps will reveal on the next play/toggle.
  function append(events: BuildEvent[]): void {
    if (events.length === 0) return;
    const wasDone = step >= sequence.length;
    for (const ev of events) sequence.push(ev);
    anchorTime = performance.now();
    anchorStep = step;
    if (wasDone && timer === null) {
      timer = window.setInterval(tick, FRAME_MS);
    }
    report();
  }

  // Fast-forward internal state to the end with no apply/report (see interface).
  function complete(): void {
    stop();
    for (; step < sequence.length; step++) {
      const ev = sequence[step];
      if (ev.kind === "node") nodes.push(ev.node);
      else edges.push(ev.edge);
    }
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
    append,
    complete,
    isPlaying: () => timer !== null,
    isDone: () => step >= sequence.length,
  };
}
