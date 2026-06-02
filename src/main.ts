import ForceGraph3D from "3d-force-graph";
import type { NodeObject, LinkObject } from "3d-force-graph";
import type { GraphNode, GraphEdge } from "./types";
import { nodeColor, nodeSize, edgeColor } from "./colors";
import { buildSequence, createBuildPlayer, type BuildPlayer } from "./build-player";
import { loadBuildSource, type BuildSource } from "./doc-source";

// 3d-force-graph's accessors hand back the library's NodeObject/LinkObject; our
// schema props ride along on the same objects, so we narrow with a cast.
const asNode = (n: NodeObject): GraphNode => n as unknown as GraphNode;
const asLink = (l: LinkObject): GraphEdge => l as unknown as GraphEdge;

// After graphData() ingests the fixture, link.source/target are replaced by the
// resolved node objects; before that they're id strings. Handle both.
const idOf = (ref: string | NodeObject | undefined): string =>
  typeof ref === "object" && ref !== null ? asNode(ref).id : String(ref);

const withAlpha = (hex: string, a: number): string => {
  const h = hex.replace("#", "");
  const r = parseInt(h.slice(0, 2), 16);
  const g = parseInt(h.slice(2, 4), 16);
  const b = parseInt(h.slice(4, 6), 16);
  return `rgba(${r},${g},${b},${a})`;
};

const container = document.getElementById("graph")!;
const statsEl = document.getElementById("stats")!;
const searchEl = document.getElementById("search") as HTMLInputElement;
const searchCountEl = document.getElementById("search-count")!;
const resetEl = document.getElementById("reset") as HTMLButtonElement;
const playPauseEl = document.getElementById("playpause") as HTMLButtonElement;
const replayEl = document.getElementById("replay") as HTMLButtonElement;
const buildProgressEl = document.getElementById("build-progress")!;
const buildBarFillEl = document.getElementById("build-bar-fill")!;
const openDocEl = document.getElementById("open-doc") as HTMLButtonElement;
const fileInputEl = document.getElementById("file-input") as HTMLInputElement;
const dropHintEl = document.getElementById("drop-hint")!;
const speedEl = document.getElementById("speed") as HTMLInputElement;

// Search/highlight state. When a search is active, matched nodes keep full color
// and the rest dim out, so a query reads as "light up the matches" against the
// dark graph.
const matched = new Set<string>();
let searchActive = false;

const graph = new ForceGraph3D(container, { controlType: "orbit" })
  .width(window.innerWidth)
  .height(window.innerHeight)
  .backgroundColor("#05070d")
  // Starts empty; the build player (A7) streams the spine in so the graph grows
  // on screen. A8 will feed this same path from the live Tauri walker stream.
  .graphData({ nodes: [], links: [] })
  .nodeColor((n) => {
    const g = asNode(n);
    const base = nodeColor(g.kind);
    if (!searchActive) return base;
    return matched.has(g.id) ? base : withAlpha(base, 0.06);
  })
  .nodeVal((n) => nodeSize(asNode(n).kind))
  .nodeLabel((n) => {
    const g = asNode(n);
    return `<b>${g.label}</b><br/><span style="opacity:.7">${g.kind}</span>`;
  })
  .nodeOpacity(0.95)
  .onNodeClick((n) => focusNode(n))
  .linkColor((l) => {
    const e = asLink(l);
    const base = edgeColor(e.kind);
    if (!searchActive) return base;
    const lit = matched.has(idOf(e.source)) && matched.has(idOf(e.target));
    return lit ? base : withAlpha(base, 0.04);
  })
  .linkWidth((l) => (asLink(l).provenance === "semantic" ? 1.2 : 0.4))
  .linkOpacity(0.5)
  // Semantic edges (the LLM-inferred meaning) animate with flowing particles;
  // the deterministic structural spine stays static — motion = "inferred".
  .linkDirectionalParticles((l) => (asLink(l).provenance === "semantic" ? 2 : 0))
  .linkDirectionalParticleSpeed(0.006)
  .linkDirectionalParticleWidth(1.4)
  // D1 fluidity: every graphData() during the streamed build reheats the force
  // sim, so on a large doc the layout was thrashing. A touch more friction calms
  // the jitter and a finite cooldown lets it settle instead of running forever;
  // the bigger win is throttling the apply rate (applyIntervalForSize, below).
  .d3VelocityDecay(0.45)
  .cooldownTime(12000)
  .warmupTicks(0);

// Re-assigning the accessors is how 3d-force-graph is told to re-evaluate node /
// link materials after the highlight state changes.
function refresh(): void {
  graph.nodeColor(graph.nodeColor()).linkColor(graph.linkColor());
}

// Ease the camera to look at a node from a fixed standoff distance.
function focusNode(node: NodeObject): void {
  const x = node.x ?? 0;
  const y = node.y ?? 0;
  const z = node.z ?? 0;
  const dist = Math.hypot(x, y, z) || 1;
  const ratio = 1 + 40 / dist;
  graph.cameraPosition({ x: x * ratio, y: y * ratio, z: z * ratio }, { x, y, z }, 900);
}

function runSearch(): void {
  const q = searchEl.value.trim().toLowerCase();
  searchActive = q.length > 0;
  matched.clear();
  if (searchActive) {
    // Search what's currently on screen, so mid-build a query only finds nodes
    // that have already been revealed.
    for (const ro of graph.graphData().nodes) {
      const n = asNode(ro);
      const hay = `${n.label} ${n.text ?? ""} ${n.kind}`.toLowerCase();
      if (hay.includes(q)) matched.add(n.id);
    }
  }
  refresh();
  searchCountEl.textContent = searchActive
    ? `${matched.size} match${matched.size === 1 ? "" : "es"}`
    : "";
  // A single hit is unambiguous — fly to it.
  if (matched.size === 1) {
    const id = [...matched][0];
    const hit = graph.graphData().nodes.find((n) => asNode(n).id === id);
    if (hit) focusNode(hit);
  }
}

function resetView(): void {
  searchEl.value = "";
  searchActive = false;
  matched.clear();
  searchCountEl.textContent = "";
  refresh();
  graph.zoomToFit(800, 60);
}

searchEl.addEventListener("input", runSearch);
searchEl.addEventListener("keydown", (e) => {
  if (e.key === "Enter" && matched.size > 0) {
    const id = [...matched][0];
    const hit = graph.graphData().nodes.find((n) => asNode(n).id === id);
    if (hit) focusNode(hit);
  }
});
resetEl.addEventListener("click", resetView);

window.addEventListener("resize", () => {
  graph.width(window.innerWidth).height(window.innerHeight);
});

// --- A8 / C1: animated build from a live source, with re-ingest -------------
// A "source" is resolved asynchronously and is origin-agnostic: in the Tauri
// shell it's the native Rust walk; in a plain browser it's the same walker via
// WASM; the baked fixture is the last-resort fallback. C1 lets the user replace
// the document at runtime (Open / drag-drop), so the build is rebuildable: each
// new document tears down the old player and starts a fresh streamed build.
const ICON_PLAY = "▶";
const ICON_PAUSE = "⏸";
const ICON_REPLAY = "⟳";

// The single live player. Re-pointed every time a new document is ingested; the
// chip controls below read it through this binding (null until the first build).
let player: BuildPlayer | null = null;

// Build-animation pacing (ms per step). The walk itself is instant — this only
// throttles the on-screen reveal. The player batches by elapsed time (one
// graphData() apply per frame), so the fast end is no longer pinned to the
// per-step render cost: MIN packs ~30 steps into each frame, completing the
// whole build in a handful of frames (≈ instant). MAX is the original deliberate
// drip. Persisted across rebuilds.
const MIN_INTERVAL_MS = 0.5;
const MAX_INTERVAL_MS = 220;
let currentIntervalMs = MAX_INTERVAL_MS;

// Slider 0 → slowest (MAX), 100 → fastest (MIN). Geometric, not linear: perceived
// speed is multiplicative, so equal slider steps feel like equal speed changes.
function sliderToInterval(v: number): number {
  const t = Math.min(1, Math.max(0, v / 100));
  return Math.round(MAX_INTERVAL_MS * (MIN_INTERVAL_MS / MAX_INTERVAL_MS) ** t);
}

// D1 — LOD apply throttle. Each graph.graphData() call reheats the entire force
// simulation, so re-applying on every revealed batch makes a large doc drop
// frames (the 37 KB stutter). Coalesce applies into a minimum interval that
// grows with node count: small graphs still repaint every frame (interval 0),
// big graphs at most a few times a second. A trailing flush (in startBuild)
// guarantees the final, complete state always lands regardless of throttling.
function applyIntervalForSize(n: number): number {
  if (n < 150) return 0;
  if (n < 400) return 90;
  if (n < 800) return 170;
  return 260;
}

// Cancels any pending trailing apply from the *previous* build before a new one
// starts, so a late-firing timer can't write stale nodes into the fresh graph.
let cancelPendingApply: (() => void) | null = null;

function clearSearch(): void {
  searchEl.value = "";
  searchActive = false;
  matched.clear();
  searchCountEl.textContent = "";
  refresh();
}

// Tear down any running build and stream a fresh one from `source`. Everything
// that differs per-document (counts, sequence, dev handles) flows from here, so
// the upload and drag-drop paths converge on this one function.
function startBuild(source: BuildSource): void {
  player?.pause();
  cancelPendingApply?.(); // drop any trailing apply still queued from the old build
  cancelPendingApply = null;
  clearSearch();
  graph.graphData({ nodes: [], links: [] });
  statsEl.textContent = `${source.nodeCount} nodes · ${source.edgeCount} edges (${source.origin})`;

  // D1 — coalesced apply. commit() is the single point that writes the revealed
  // subgraph into the scene; the player can call apply() every frame but on a
  // large doc we throttle the actual graphData() to applyIntervalForSize(), with
  // a trailing timer so the latest (superset) state always lands. flushApply()
  // forces the pending write out immediately (used on build completion).
  let lastApplyAt = 0;
  let trailingTimer: number | null = null;
  let pending: { nodes: GraphNode[]; edges: GraphEdge[] } | null = null;

  const commit = (nodes: GraphNode[], edges: GraphEdge[]): void => {
    lastApplyAt = performance.now();
    graph.graphData({
      nodes: nodes as unknown as NodeObject[],
      links: edges as unknown as LinkObject[],
    });
  };
  const flushApply = (): void => {
    if (trailingTimer !== null) {
      window.clearTimeout(trailingTimer);
      trailingTimer = null;
    }
    if (pending) {
      commit(pending.nodes, pending.edges);
      pending = null;
    }
  };
  cancelPendingApply = () => {
    if (trailingTimer !== null) {
      window.clearTimeout(trailingTimer);
      trailingTimer = null;
    }
    pending = null;
  };

  // onProgress fires ~every frame and steps arrive in batches, so re-framing is
  // throttled by wall-clock too — refit at most ~4×/s during the build (it's
  // expensive, and firing it every frame would erase the apply-batching win).
  let lastFitAt = 0;
  const p = createBuildPlayer({
    sequence: source.sequence,
    intervalMs: currentIntervalMs,
    apply: (nodes, edges) => {
      const interval = applyIntervalForSize(nodes.length);
      if (interval === 0) {
        commit(nodes, edges);
        return;
      }
      pending = { nodes, edges };
      if (trailingTimer !== null) return; // newest state is parked in `pending`
      const elapsed = performance.now() - lastApplyAt;
      if (elapsed >= interval) {
        flushApply();
      } else {
        trailingTimer = window.setTimeout(() => {
          trailingTimer = null;
          flushApply();
        }, interval - elapsed);
      }
    },
    onProgress: (step, total, done) => {
      const pct = total === 0 ? 0 : Math.round((step / total) * 100);
      buildBarFillEl.style.width = `${done ? 100 : pct}%`;
      playPauseEl.textContent = p.isPlaying() ? ICON_PAUSE : done ? ICON_REPLAY : ICON_PLAY;
      buildProgressEl.textContent = done ? "ready" : `${step}/${total}`;
      // Keep the growing graph framed while the build runs; settle on completion.
      const now = performance.now();
      if (done) {
        flushApply(); // make sure the final batch is on screen before framing it
        graph.zoomToFit(800, 80);
        lastFitAt = now;
      } else if (now - lastFitAt > 250) {
        graph.zoomToFit(500, 80);
        lastFitAt = now;
      }
    },
  });
  player = p;
  p.play();

  // Dev-only handle so the running 3D scene is inspectable from the page console
  // / preview tooling (WebGL canvases can't be verified via readPixels under the
  // default preserveDrawingBuffer:false). Stripped from production builds.
  if (import.meta.env.DEV) {
    Object.assign(window as object, {
      __doctreeGraph: graph,
      __doctreePlayer: p,
      __doctreeSequence: source.sequence,
      __doctreeBuildSequence: buildSequence,
      __doctreeSource: source,
    });
  }
}

// Resolve the walk for `text` (or the bundled default), then build. Walking can
// be slow on a big document, so we surface a "walking…" state before the await.
async function loadAndBuild(text?: string, label?: string): Promise<void> {
  statsEl.textContent = label ? `walking ${label}…` : "walking…";
  try {
    const source = await loadBuildSource(text);
    startBuild(source);
  } catch (err) {
    console.error("walk failed:", err);
    statsEl.textContent = "walk failed — see console";
  }
}

// Read a dropped/selected file as text and rebuild from it. Phase 1 only ingests
// plain text / markdown, so a naive readAsText is exactly right.
function ingestFile(file: File): void {
  const reader = new FileReader();
  reader.onload = () => {
    const text = typeof reader.result === "string" ? reader.result : "";
    void loadAndBuild(text, file.name);
  };
  reader.onerror = () => {
    statsEl.textContent = `could not read ${file.name}`;
  };
  reader.readAsText(file);
}

// Chip controls are wired once; they act on whatever the current `player` is.
playPauseEl.addEventListener("click", () => {
  if (!player) return;
  // After completion the button replays; otherwise it toggles play/pause.
  if (player.isDone()) player.replay();
  else player.toggle();
  playPauseEl.textContent = player.isPlaying() ? ICON_PAUSE : ICON_PLAY;
});
replayEl.addEventListener("click", () => {
  if (!player) return;
  clearSearch();
  player.replay();
});

// Speed slider re-paces the current build live and is remembered for the next
// one. Sync the starting pace from the slider's initial position so the HTML
// default and currentIntervalMs can't drift apart.
currentIntervalMs = sliderToInterval(Number(speedEl.value));
speedEl.addEventListener("input", () => {
  currentIntervalMs = sliderToInterval(Number(speedEl.value));
  player?.setSpeed(currentIntervalMs);
});

// Upload: the visible button proxies the hidden <input type=file>; resetting its
// value after each pick lets the user re-select the same file to re-walk it.
openDocEl.addEventListener("click", () => fileInputEl.click());
fileInputEl.addEventListener("change", () => {
  const file = fileInputEl.files?.[0];
  if (file) ingestFile(file);
  fileInputEl.value = "";
});

// Drag-drop anywhere on the window. dragenter/leave nest, so we count depth and
// only hide the hint when the last overlapping element is left.
let dragDepth = 0;
window.addEventListener("dragenter", (e) => {
  e.preventDefault();
  dragDepth += 1;
  dropHintEl.classList.add("active");
});
window.addEventListener("dragover", (e) => e.preventDefault());
window.addEventListener("dragleave", (e) => {
  e.preventDefault();
  dragDepth = Math.max(0, dragDepth - 1);
  if (dragDepth === 0) dropHintEl.classList.remove("active");
});
window.addEventListener("drop", (e) => {
  e.preventDefault();
  dragDepth = 0;
  dropHintEl.classList.remove("active");
  const file = e.dataTransfer?.files?.[0];
  if (file) ingestFile(file);
});

void loadAndBuild();
