import ForceGraph3D from "3d-force-graph";
import type { NodeObject, LinkObject } from "3d-force-graph";
import type { GraphNode, GraphEdge } from "./types";
import { nodeColor, nodeSize, edgeColor } from "./colors";
import { buildSequence, createBuildPlayer } from "./build-player";
import { loadBuildSource } from "./doc-source";

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
  .linkDirectionalParticleWidth(1.4);

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

// --- A8: animated build from the live source (Tauri walker, or fixture) ------
// The sequence is resolved asynchronously: in the Tauri shell it's the live
// Rust walk of the document; in a plain browser it's the baked fixture. Either
// way it's the same BuildEvent[] the player consumes, so everything below the
// await is origin-agnostic.
async function initBuild(): Promise<void> {
  const source = await loadBuildSource();
  statsEl.textContent = `${source.nodeCount} nodes · ${source.edgeCount} edges (${source.origin})`;

  const player = createBuildPlayer({
    sequence: source.sequence,
    intervalMs: 220,
    apply: (nodes, edges) => {
      graph.graphData({
        nodes: nodes as unknown as NodeObject[],
        links: edges as unknown as LinkObject[],
      });
    },
    onProgress: (step, total, done) => {
      playPauseEl.textContent = player.isPlaying() ? "Pause" : done ? "Replay" : "Play";
      buildProgressEl.textContent = done
        ? "build complete — explore"
        : `building ${step}/${total}`;
      // Keep the growing graph framed while the build runs; settle on completion.
      if (done) graph.zoomToFit(800, 80);
      else if (step % 4 === 0) graph.zoomToFit(500, 80);
    },
  });

  playPauseEl.addEventListener("click", () => {
    // After completion the button replays; otherwise it toggles play/pause.
    if (player.isDone()) player.replay();
    else player.toggle();
    playPauseEl.textContent = player.isPlaying() ? "Pause" : "Play";
  });
  replayEl.addEventListener("click", () => {
    searchEl.value = "";
    searchActive = false;
    matched.clear();
    searchCountEl.textContent = "";
    refresh();
    player.replay();
  });

  player.play();

  // Dev-only handle so the running 3D scene is inspectable from the page console
  // / preview tooling (WebGL canvases can't be verified via readPixels under the
  // default preserveDrawingBuffer:false). Stripped from production builds.
  if (import.meta.env.DEV) {
    Object.assign(window as object, {
      __doctreeGraph: graph,
      __doctreePlayer: player,
      __doctreeSequence: source.sequence,
      __doctreeBuildSequence: buildSequence,
      __doctreeSource: source,
    });
  }
}

void initBuild();
