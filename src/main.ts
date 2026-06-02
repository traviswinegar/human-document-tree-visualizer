import ForceGraph3D from "3d-force-graph";
import type { NodeObject, LinkObject } from "3d-force-graph";
import { Vector2 } from "three";
import { UnrealBloomPass } from "three/addons/postprocessing/UnrealBloomPass.js";
import { OutputPass } from "three/addons/postprocessing/OutputPass.js";
import type { GraphNode, GraphEdge, NodeKind, Provenance } from "./types";
import { nodeColor, nodeSize, edgeColor } from "./colors";
import { buildSequence, createBuildPlayer, type BuildPlayer } from "./build-player";
import {
  loadBuildSource,
  searchByMeaning,
  defaultDocument,
  type BuildSource,
  type ResolvedPipeline,
} from "./doc-source";

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
const meaningSearchEl = document.getElementById("meaning-search") as HTMLInputElement;
const meaningCountEl = document.getElementById("meaning-count")!;
const routingEl = document.getElementById("routing")!;
const resetEl = document.getElementById("reset") as HTMLButtonElement;
const playPauseEl = document.getElementById("playpause") as HTMLButtonElement;
const replayEl = document.getElementById("replay") as HTMLButtonElement;
const buildProgressEl = document.getElementById("build-progress")!;
const buildBarFillEl = document.getElementById("build-bar-fill")!;
const openDocEl = document.getElementById("open-doc") as HTMLButtonElement;
const fileInputEl = document.getElementById("file-input") as HTMLInputElement;
const dropHintEl = document.getElementById("drop-hint")!;
const speedEl = document.getElementById("speed") as HTMLInputElement;
const sidebarDocEl = document.getElementById("sidebar-doc")!;
const sidebarCollapseEl = document.getElementById("sidebar-collapse") as HTMLButtonElement;
const sidebarReopenEl = document.getElementById("sidebar-reopen") as HTMLButtonElement;
const nodeDetailsEl = document.getElementById("node-details")!;

// Search/highlight state. When a search is active, matched nodes keep full color
// and the rest dim out, so a query reads as "light up the matches" against the
// dark graph. Both the literal search and the B4 "find by meaning" search write
// to this one channel (they're mutually exclusive views of the same highlight).
const matched = new Set<string>();
let searchActive = false;

// The text of the document currently on screen. Tracked so the B4 meaning search
// (which re-walks + embeds the same text on the backend) has something to query.
let currentText = defaultDocument;

// Human labels for the resolved pipeline shown on the B5 routing line.
const PIPELINE_LABEL: Record<ResolvedPipeline, string> = {
  semantic_build: "hybrid (LLM)",
  embedded_build: "similarity",
  structural_build: "structural",
};

// Selection/highlight state (D2). Clicking a node lights it, its incident edges
// and immediate neighbors and dims the rest — through the *same* dimming channel
// as search, so the two coexist (a node lit by either stays lit). `neighborIds`
// is recomputed per click from whatever edges are currently on screen.
let selectedId: string | null = null;
const neighborIds = new Set<string>();

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
    if (!highlightActive()) return base;
    // Dimmed context keeps a faint floor (was 0.06 — so low it vanished on a big
    // graph) so you can still read the surrounding shape while the matches glow.
    return isNodeLit(g.id) ? base : withAlpha(base, 0.16);
  })
  .nodeVal((n) => nodeSize(asNode(n).kind))
  .nodeLabel((n) => {
    const g = asNode(n);
    return `<b>${g.label}</b><br/><span style="opacity:.7">${g.kind}</span>`;
  })
  .nodeOpacity(1)
  .onNodeClick((n) => selectNode(n))
  .onBackgroundClick(() => clearSelection())
  .linkColor((l) => {
    const e = asLink(l);
    const base = edgeColor(e.kind);
    if (!highlightActive()) return base;
    return isLinkLit(e) ? base : withAlpha(base, 0.08);
  })
  // Incident edges of the selected node thicken so the local neighborhood reads
  // as a unit; otherwise semantic edges stay slightly bolder than structural.
  .linkWidth((l) => {
    const e = asLink(l);
    if (selectedId !== null) {
      if (idOf(e.source) === selectedId || idOf(e.target) === selectedId) return 2;
    }
    return e.provenance === "semantic" ? 1.2 : 0.4;
  })
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

// --- Bloom glow ------------------------------------------------------------
// The single biggest "the graph is so dark" lever: an UnrealBloom pass makes the
// bright node spheres bleed light against the near-black background, so the graph
// reads as a luminous nebula instead of flat dots. 3d-force-graph builds its
// post-processing composer with just a RenderPass; we append bloom, then an
// OutputPass to do the final sRGB/tone-map conversion (bloom must run in linear
// space *before* that). The ESM 3d-force-graph externalises `three`, so these
// addon passes share the one `three` instance the composer renders with.
// Tunables — bump STRENGTH for more glow, lower THRESHOLD to make dimmer nodes
// (and edges) bloom too:
const BLOOM_STRENGTH = 0.85; // intensity of the glow
const BLOOM_RADIUS = 0.55; // how far the glow spreads
const BLOOM_THRESHOLD = 0.08; // luminance above which a pixel blooms (low → most nodes glow)
const bloomPass = new UnrealBloomPass(
  new Vector2(window.innerWidth, window.innerHeight),
  BLOOM_STRENGTH,
  BLOOM_RADIUS,
  BLOOM_THRESHOLD
);
const composer = graph.postProcessingComposer();
composer.addPass(bloomPass);
composer.addPass(new OutputPass());

// Re-assigning the accessors is how 3d-force-graph is told to re-evaluate node /
// link materials after the highlight state changes.
function refresh(): void {
  graph.nodeColor(graph.nodeColor()).linkColor(graph.linkColor()).linkWidth(graph.linkWidth());
}

// Is any dimming mode on? When neither search nor selection is active every
// node/edge renders at full colour (no dimming pass).
function highlightActive(): boolean {
  return searchActive || selectedId !== null;
}
// A node stays lit if it matches the active search OR is the selection / one of
// its neighbours. (Both modes union, so they can be on at once.)
function isNodeLit(id: string): boolean {
  if (searchActive && matched.has(id)) return true;
  if (selectedId !== null && (id === selectedId || neighborIds.has(id))) return true;
  return false;
}
// An edge stays lit if both endpoints match the search, or it is incident to the
// selected node.
function isLinkLit(e: GraphEdge): boolean {
  const s = idOf(e.source);
  const t = idOf(e.target);
  if (searchActive && matched.has(s) && matched.has(t)) return true;
  if (selectedId !== null && (s === selectedId || t === selectedId)) return true;
  return false;
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

// D2 — select a node: light it, its incident edges and immediate neighbours,
// dim everything else, and ease the camera to it. Neighbours are read from the
// edges currently on screen, so mid-build a selection only reflects what's been
// revealed so far.
function selectNode(node: NodeObject): void {
  const id = asNode(node).id;
  selectedId = id;
  neighborIds.clear();
  for (const l of graph.graphData().links) {
    const e = asLink(l);
    const s = idOf(e.source);
    const t = idOf(e.target);
    if (s === id) neighborIds.add(t);
    else if (t === id) neighborIds.add(s);
  }
  refresh();
  focusNode(node);
  jumpToNode(id); // D4 — light up its line(s) in the sidebar…
  showDetails(asNode(node)); // …and surface its details
}

// Select by id — used by the sidebar text and the details-panel connections so
// the document and the graph stay two views of the same selection.
function selectNodeById(id: string): void {
  const ro = graph.graphData().nodes.find((x) => asNode(x).id === id);
  if (ro) selectNode(ro);
}

// Drop the selection highlight (background click / Escape / new build).
function clearSelection(): void {
  if (selectedId === null) return;
  selectedId = null;
  neighborIds.clear();
  hideDetails();
  clearDocActive();
  refresh();
}

// --- D3: sidebar document reconstruction ------------------------------------
const esc = (s: string): string =>
  s.replace(
    /[&<>"]/g,
    (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c] ?? c
  );

const byStart = (a: GraphNode, b: GraphNode): number =>
  (a.span?.start ?? 0) - (b.span?.start ?? 0);

// Rebuild the unfolding document from the text-bearing nodes revealed so far.
// Only sections (headings) and sentences tile the source cleanly — clauses,
// quotes and references are sub-spans *inside* sentences and would duplicate
// text, so they're left out here. Sentences are grouped into their paragraph by
// span containment (the live walker stamps paragraphs with byte spans); any
// sentence not contained by a revealed paragraph — e.g. the hand-authored
// fixture, whose paragraphs carry no span — falls back into one ordered block.
// Rebuilt on every (throttled) commit, so the text grows in step with the graph.
function renderSidebar(nodes: GraphNode[]): void {
  const hasSpan = (n: GraphNode): boolean => !!n.span && typeof n.span.start === "number";
  const sections = nodes.filter((n) => n.kind === "section" && hasSpan(n));
  const paragraphs = nodes.filter((n) => n.kind === "paragraph" && hasSpan(n));
  const sentences = nodes
    .filter((n) => n.kind === "sentence" && hasSpan(n) && !!n.text)
    .sort(byStart);

  const paraOf = new Map<string, GraphNode[]>();
  for (const p of paragraphs) paraOf.set(p.id, []);
  const orphans: GraphNode[] = [];
  for (const se of sentences) {
    const s = se.span!.start;
    const p = paragraphs.find((pp) => s >= pp.span!.start && s < pp.span!.end);
    if (p) paraOf.get(p.id)!.push(se);
    else orphans.push(se);
  }

  type Block =
    | { start: number; type: "section"; node: GraphNode }
    | { start: number; type: "para"; sentences: GraphNode[] };
  const blocks: Block[] = [];
  for (const sec of sections) blocks.push({ start: sec.span!.start, type: "section", node: sec });
  for (const p of paragraphs)
    blocks.push({ start: p.span!.start, type: "para", sentences: paraOf.get(p.id)! });
  if (orphans.length)
    blocks.push({ start: orphans[0].span!.start, type: "para", sentences: orphans });
  blocks.sort((a, b) => a.start - b.start);

  let html = "";
  for (const b of blocks) {
    if (b.type === "section") {
      html += `<h3 class="doc-section" data-node-id="${esc(b.node.id)}">${esc(b.node.label)}</h3>`;
    } else {
      if (b.sentences.length === 0) continue;
      html += `<p class="doc-para">`;
      for (const se of b.sentences)
        html += `<span class="doc-sentence" data-node-id="${esc(se.id)}">${esc(se.text!)}</span> `;
      html += `</p>`;
    }
  }
  sidebarDocEl.innerHTML = html || '<div class="doc-empty">…</div>';
}

// --- D3: layout — the 3D canvas yields width to the sidebar -----------------
const SIDEBAR_W = 360;
let sidebarOpen = true;

function layoutGraph(): void {
  const w = sidebarOpen ? window.innerWidth - SIDEBAR_W : window.innerWidth;
  graph.width(Math.max(320, w)).height(window.innerHeight);
}

function setSidebar(open: boolean): void {
  sidebarOpen = open;
  document.body.classList.toggle("sidebar-collapsed", !open);
  layoutGraph();
  graph.zoomToFit(600, 80);
}

// --- D4: jump-to-text + node details panel ----------------------------------
function clearDocActive(): void {
  sidebarDocEl
    .querySelectorAll(".doc-active")
    .forEach((el) => el.classList.remove("doc-active"));
}

// Light up (and scroll to) the clicked node's line in the sidebar. Sentences and
// headings have their own line; everything else (paragraphs, clauses/quotes/
// refs, terms, and — once the LLM layer lands — entities) has no line of its
// own, so we highlight the sentences it connects to instead, sentences first.
function jumpToNode(id: string): void {
  clearDocActive();
  const targets: Element[] = [];
  const own = sidebarDocEl.querySelector(`[data-node-id="${id}"]`);
  if (own) {
    targets.push(own);
  } else {
    const bySentenceFirst = [...neighborIds].sort(
      (a, b) => (a.startsWith("sent:") ? 0 : 1) - (b.startsWith("sent:") ? 0 : 1)
    );
    for (const nid of bySentenceFirst) {
      const el = sidebarDocEl.querySelector(`[data-node-id="${nid}"]`);
      if (el) targets.push(el);
    }
  }
  if (targets.length === 0) return;
  for (const el of targets) el.classList.add("doc-active");
  targets[0].scrollIntoView({ behavior: "smooth", block: "center" });
}

const PROV_META: Record<Provenance, { dot: string; text: string }> = {
  structural: { dot: "#5f7fbf", text: "structural · deterministic walk" },
  semantic: { dot: "#ff6b6b", text: "semantic · LLM-inferred" },
  embedding: { dot: "#06d6a0", text: "embedding · similarity" },
};

// Populate the details panel: kind chip, label, provenance, the node's own text
// (if it adds anything beyond the label), and its connections grouped by edge
// kind + direction. Each connection is clickable to navigate the selection.
function showDetails(node: GraphNode): void {
  const nodesById = new Map<string, GraphNode>();
  for (const ro of graph.graphData().nodes) {
    const n = asNode(ro);
    nodesById.set(n.id, n);
  }

  type Conn = { id: string; label: string; kind: NodeKind };
  const groups = new Map<string, Conn[]>();
  for (const l of graph.graphData().links) {
    const e = asLink(l);
    const s = idOf(e.source);
    const t = idOf(e.target);
    let otherId: string | null = null;
    let arrow = "";
    if (s === node.id) {
      otherId = t;
      arrow = "→";
    } else if (t === node.id) {
      otherId = s;
      arrow = "←";
    }
    if (otherId === null) continue;
    const other = nodesById.get(otherId);
    if (!other) continue;
    const key = `${e.kind} ${arrow}`;
    if (!groups.has(key)) groups.set(key, []);
    groups.get(key)!.push({ id: other.id, label: other.label, kind: other.kind });
  }

  const prov = PROV_META[node.provenance] ?? PROV_META.structural;
  let html = "";
  html += `<div class="nd-head">`;
  html += `<span class="nd-kind" style="background:${nodeColor(node.kind)}">${esc(node.kind)}</span>`;
  html += `<span class="nd-label">${esc(node.label)}</span>`;
  html += `<button id="node-details-close" type="button" title="Close (Esc)" aria-label="Close">×</button>`;
  html += `</div>`;
  html += `<div class="nd-prov"><span class="dot" style="background:${prov.dot}"></span>${esc(prov.text)}</div>`;
  if (node.text && node.text !== node.label) {
    html += `<div class="nd-text">${esc(node.text)}</div>`;
  }
  for (const key of [...groups.keys()].sort()) {
    const conns = groups.get(key)!;
    html += `<div class="nd-conn-group"><div class="nd-conn-kind">${esc(key)} (${conns.length})</div>`;
    const seen = new Set<string>();
    let shown = 0;
    for (const c of conns) {
      if (seen.has(c.id)) continue;
      seen.add(c.id);
      if (shown >= 40) {
        html += `<span class="nd-conn-kind">…</span>`;
        break;
      }
      html += `<span class="nd-conn" data-node-id="${esc(c.id)}"><span class="swatch" style="background:${nodeColor(c.kind)}"></span>${esc(c.label)}</span>`;
      shown += 1;
    }
    html += `</div>`;
  }

  nodeDetailsEl.innerHTML = html;
  nodeDetailsEl.classList.remove("hidden");
}

function hideDetails(): void {
  nodeDetailsEl.classList.add("hidden");
  nodeDetailsEl.innerHTML = "";
}

// Two-way wiring: clicking a sentence/heading in the document selects its node;
// clicking a connection in the details panel navigates to that node; the close
// button drops the selection. Delegated once on the containers (their innards
// are rebuilt on every render/selection).
sidebarDocEl.addEventListener("click", (e) => {
  const el = (e.target as HTMLElement).closest("[data-node-id]");
  if (el) selectNodeById(el.getAttribute("data-node-id")!);
});
nodeDetailsEl.addEventListener("click", (e) => {
  const target = e.target as HTMLElement;
  if (target.closest("#node-details-close")) {
    clearSelection();
    return;
  }
  const conn = target.closest(".nd-conn");
  if (conn) selectNodeById(conn.getAttribute("data-node-id")!);
});

function runSearch(): void {
  // Literal search owns the highlight channel while active; drop any stale
  // meaning-search hits so the two never blend.
  meaningSearchEl.value = "";
  meaningCountEl.textContent = "";
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
  meaningSearchEl.value = "";
  meaningCountEl.textContent = "";
  searchActive = false;
  matched.clear();
  searchCountEl.textContent = "";
  selectedId = null;
  neighborIds.clear();
  hideDetails();
  clearDocActive();
  refresh();
  graph.zoomToFit(800, 60);
}

// B4 "find by meaning": rank the document's nodes by embedding similarity to the
// query (desktop + `vectordb` only — the box is hidden otherwise) and light the
// hits through the same dimming channel as the literal search. Each call is an
// embedding round-trip, so it fires on Enter, not per keystroke.
async function runMeaningSearch(): Promise<void> {
  const q = meaningSearchEl.value.trim();
  if (q.length === 0) {
    // Clearing the box drops its highlight.
    matched.clear();
    searchActive = false;
    meaningCountEl.textContent = "";
    refresh();
    return;
  }
  meaningCountEl.textContent = "…";
  try {
    const hits = await searchByMeaning(q, currentText);
    // Take over the highlight channel from any literal search.
    searchEl.value = "";
    searchCountEl.textContent = "";
    matched.clear();
    for (const h of hits) matched.add(h.id);
    searchActive = matched.size > 0;
    refresh();
    meaningCountEl.textContent = `${hits.length} hit${hits.length === 1 ? "" : "s"}`;
    // Fly to the best (already-revealed) hit, if any.
    if (hits.length > 0) {
      const best = graph.graphData().nodes.find((n) => asNode(n).id === hits[0].id);
      if (best) focusNode(best);
    }
  } catch (err) {
    console.error("meaning search failed:", err);
    meaningCountEl.textContent = "search failed";
  }
}

// B5 routing line under the stats: the document class + confidence + the pipeline
// actually run, plus a warm hint when the ideal pipeline wasn't compiled in (or a
// model was missing and we fell back to the spine). On the browser/WASM path
// there's no classifier, so it reads as the in-browser structural walk.
function renderRouting(source: BuildSource): void {
  const r = source.routing;
  if (!r) {
    routingEl.textContent = source.origin === "fixture" ? "" : "structural · in-browser walk";
    return;
  }
  const conf = Math.round(r.confidence * 100);
  const pipeline = source.fellBack ? "structural" : PIPELINE_LABEL[r.resolvedPipeline];
  let html = `<span class="cls">${r.class}</span> · ${conf}% · ${pipeline}`;
  if (source.fellBack) {
    html += ` <span class="hint">· no model on disk; using the spine</span>`;
  } else if (r.downgraded) {
    const want =
      r.recommendedPipeline === "narrative_hybrid" ? "--features llm" : "--features vectordb";
    html += ` <span class="hint">· rebuild with ${want} for richer extraction</span>`;
  }
  routingEl.innerHTML = html;
}

searchEl.addEventListener("input", runSearch);
searchEl.addEventListener("keydown", (e) => {
  if (e.key === "Enter" && matched.size > 0) {
    const id = [...matched][0];
    const hit = graph.graphData().nodes.find((n) => asNode(n).id === id);
    if (hit) focusNode(hit);
  }
});
// Meaning search fires on Enter (each call is a backend embedding round-trip);
// emptying the box clears its highlight immediately.
meaningSearchEl.addEventListener("keydown", (e) => {
  if (e.key === "Enter") void runMeaningSearch();
});
meaningSearchEl.addEventListener("input", () => {
  if (meaningSearchEl.value.trim() === "") void runMeaningSearch();
});
resetEl.addEventListener("click", resetView);

// Escape clears the click-selection highlight (background click does too).
window.addEventListener("keydown", (e) => {
  if (e.key === "Escape") clearSelection();
});

// Sidebar collapse/reopen; the graph re-fits into the reclaimed/yielded width.
sidebarCollapseEl.addEventListener("click", () => setSidebar(false));
sidebarReopenEl.addEventListener("click", () => setSidebar(true));

window.addEventListener("resize", layoutGraph);
// Narrow the canvas for the sidebar (open by default) before the first build.
layoutGraph();

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
  meaningSearchEl.value = "";
  meaningCountEl.textContent = "";
  searchActive = false;
  matched.clear();
  searchCountEl.textContent = "";
  selectedId = null;
  neighborIds.clear();
  hideDetails();
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
  sidebarDocEl.innerHTML = '<div class="doc-empty">building…</div>';
  statsEl.textContent = `${source.nodeCount} nodes · ${source.edgeCount} edges (${source.origin})`;
  renderRouting(source); // B5 — surface the class + resolved pipeline / downgrade
  // B4 — offer "find by meaning" only on a build that can actually embed.
  const meaningOn = Boolean(source.routing?.capabilities.vectordb);
  meaningSearchEl.classList.toggle("hidden", !meaningOn);
  meaningCountEl.classList.toggle("hidden", !meaningOn);

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
    renderSidebar(nodes); // D3 — unfold the document in step with the graph
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
      // Canvas-raycast clicks can't be synthesized from the console/preview, so
      // expose the click-selection (D2) by id for verification/scripting.
      __doctreeSelect: (id: string): boolean => {
        const n = graph.graphData().nodes.find((x) => asNode(x).id === id);
        if (n) selectNode(n);
        return Boolean(n);
      },
    });
  }
}

// Resolve the walk for `text` (or the bundled default), then build. Walking can
// be slow on a big document, so we surface a "walking…" state before the await.
async function loadAndBuild(text?: string, label?: string): Promise<void> {
  statsEl.textContent = label ? `walking ${label}…` : "walking…";
  currentText = text ?? defaultDocument; // remember it for the meaning search
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
