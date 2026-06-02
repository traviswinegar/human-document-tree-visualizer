import ForceGraph3D from "3d-force-graph";
import type { NodeObject, LinkObject } from "3d-force-graph";
import { Vector2 } from "three";
import { UnrealBloomPass } from "three/addons/postprocessing/UnrealBloomPass.js";
import { OutputPass } from "three/addons/postprocessing/OutputPass.js";
import type { GraphNode, GraphEdge, NodeKind, EdgeKind, Provenance } from "./types";
import { nodeColor, nodeSize, edgeColor } from "./colors";
import { buildSequence, createBuildPlayer, type BuildPlayer } from "./build-player";
import { isPdf, extractPdfText } from "./pdf";
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
const elapsedEl = document.getElementById("elapsed")!;
const statsBodyEl = document.getElementById("stats-body")!;
const statsCollapseEl = document.getElementById("stats-collapse") as HTMLButtonElement;
const statsReopenEl = document.getElementById("stats-reopen") as HTMLButtonElement;
const playbackEl = document.getElementById("playback")!;
const semanticStatusEl = document.getElementById("semantic-status")!;
const layoutEl = document.getElementById("layout") as HTMLSelectElement;
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
  .warmupTicks(0)
  // Defensive no-op: we never enable a built-in DAG layout (the document graph
  // isn't a tree — see the layout dropdown below for why), but if one is ever
  // re-introduced this keeps 3d-force-graph tolerating the cyclic cross-links
  // (co_occurs_with, similarity) instead of throwing and freezing the layout.
  .onDagError(() => {});

// --- Adaptive force tuning (don't collapse the snakes into a ball) ---------
// A force-directed graph relaxes toward its minimum-energy shape. With d3's
// default charge (-30) the structural spine — a long `precedes` chain of
// sentences plus its branching sections/clauses — coils up into one dense ball
// the moment the build stops reheating the sim. So crank the node-node repulsion
// (and give edges a little more resting room) to make the *equilibrium itself*
// open and filamentary: the shape that grows is the shape that stays.
//
// But one fixed setting can't serve both the ~40-node fixture and a 100k-word
// document (~1800 nodes, ~9000 edges): the small-graph tuning that looked good on
// the fixture leaves a large graph a cramped, unreadable knot. So scale the
// repulsion, its range, and the resting edge length with node count — small
// graphs keep the original open look, large graphs spread enough to read.
// `distanceMax` caps the repulsion range so the graph opens without exploding.
type ForceTunable = {
  strength?(v: number): unknown;
  distanceMax?(v: number): unknown;
  distance?(v: number): unknown;
};

// 0 at small graphs (≤150 nodes), ramping to 1 by ~1800. One lever for every
// size-sensitive knob below, so the whole layout scales coherently.
function spread(n: number): number {
  return Math.min(1, Math.max(0, (n - 150) / 1650));
}

// Re-tune the force simulation for a graph of `n` nodes. Called when a build's
// final size is known (and again when the semantic delta grows it) and on every
// layout switch.
function applyForceTuning(n: number): void {
  const t = spread(n);
  const charge = -(90 + 180 * t); // -90 (small) … -270 (large): node-node repulsion
  const chargeMax = 600 + 1100 * t; // 600 … 1700: cap repulsion range (no explosion)
  const linkDist = 40 + 55 * t; // 40 … 95: resting edge length
  const cf = graph.d3Force("charge") as ForceTunable | undefined;
  cf?.strength?.(charge);
  cf?.distanceMax?.(chargeMax);
  const lf = graph.d3Force("link") as ForceTunable | undefined;
  lf?.distance?.(linkDist);
}
applyForceTuning(0); // baseline for the empty / initial graph

// --- Layout dropdown -------------------------------------------------------
// Why no top-down/left-right/radial *DAG* layout? The document graph isn't a
// tree. The `precedes` edge chains every sentence to the next, so a DAG layout
// assigns each of ~1000 sentences its own successive depth level → a ~1000-level
// deep, one-node-wide column (the "thin line" a big doc collapsed into). The term
// co-occurrence and similarity edges pile cycles on top. Tuning can't unbend a
// linear chain, so instead of fighting the topology we offer layouts that suit
// it: the free force field (in 3D, or flattened to a more legible 2D map) and a
// structural "Layers" mode that pins each node's height by its *containment*
// depth and lets the force field spread each band sideways — a real hierarchy,
// driven by `part_of` alone, immune to the precedes chain and the cycles.
type LayoutMode = "force3d" | "force2d" | "layers";
let currentLayout: LayoutMode = "force3d";

// Containment depth by node kind — the structural hierarchy the walker builds via
// `part_of` (section ▸ paragraph ▸ sentence ▸ clause/quote/ref ▸ term ▸ entity).
// Stratifies the Layers view: shallow = high, deep = low.
const KIND_LAYER: Record<NodeKind, number> = {
  section: 0,
  paragraph: 1,
  sentence: 2,
  clause: 3,
  quote: 3,
  reference: 3,
  term: 4,
  character: 5,
  place: 5,
  concept: 5,
  event: 5,
  object: 5,
  group: 5,
};
const MAX_LAYER = 5;

// Vertical gap between structural bands. Scales with graph size so the bands stay
// separated as the in-band force cloud spreads wider on a big doc.
function layerGap(n: number): number {
  return Math.max(160, Math.min(1500, Math.sqrt(n) * 30));
}

// Pin each node's Y to its structural band. Fixing fy holds the height while
// leaving x/z free for the force field to spread the band into a plane — crisp
// strata instead of the force ball. Re-applied as the build streams in new nodes
// (see commit()), since freshly added nodes arrive unpinned.
function pinLayers(nodes: readonly NodeObject[]): void {
  const gap = layerGap(nodes.length);
  for (const ro of nodes) {
    const layer = KIND_LAYER[asNode(ro).kind] ?? MAX_LAYER;
    ro.fy = (MAX_LAYER / 2 - layer) * gap;
  }
}
// Release the Y pins (undefined = "not fixed" to d3) when leaving Layers mode.
function unpinLayers(nodes: readonly NodeObject[]): void {
  for (const ro of nodes) ro.fy = undefined;
}

function applyLayout(mode: LayoutMode): void {
  currentLayout = mode;
  const data = graph.graphData();
  if (mode === "layers") {
    pinLayers(data.nodes);
    graph.numDimensions(3);
  } else {
    unpinLayers(data.nodes); // clear any pins left over from a prior Layers pass
    graph.numDimensions(mode === "force2d" ? 2 : 3);
  }
  applyForceTuning(data.nodes.length);
  graph.d3ReheatSimulation();
  // Let the new layout take a few ticks, then frame it.
  window.setTimeout(() => graph.zoomToFit(700, 80), 450);
}

layoutEl.addEventListener("change", () => applyLayout(layoutEl.value as LayoutMode));

// --- Bloom glow ------------------------------------------------------------
// The single biggest "the graph is so dark" lever: an UnrealBloom pass makes the
// bright node spheres bleed light against the near-black background, so the graph
// reads as a luminous nebula instead of flat dots. 3d-force-graph builds its
// post-processing composer with just a RenderPass; we append bloom, then an
// OutputPass to do the final sRGB/tone-map conversion (bloom must run in linear
// space *before* that). The ESM 3d-force-graph externalises `three`, so these
// addon passes share the one `three` instance the composer renders with.
// Tunables — bump STRENGTH for more glow, lower THRESHOLD to make dimmer nodes
// (and edges) bloom too. Kept deliberately low: on a dense graph the glow of
// hundreds of overlapping spheres *accumulates* into a washed-out haze in the
// crowded centre, so the robust base palette (colors.ts) does the heavy lifting
// and bloom is just a faint rim on the brightest cores.
const BLOOM_STRENGTH = 0.18; // intensity of the glow — subtle accent, not the main event
const BLOOM_RADIUS = 0.2; // how tightly the glow hugs the node (smaller = tighter halo)
const BLOOM_THRESHOLD = 0.35; // only the brightest cores bloom — keeps the dense centre defined, not hazy
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

// --- #43: left panel — statistical analysis of the document's structure ------
// Computed purely from the nodes/edges currently on screen (native-free, so it
// works identically on the desktop, browser-WASM, and fixture paths) and
// re-rendered on every throttled commit, so the figures grow in step with the
// build. Kinds are walked in a fixed structural order — not sorted by count — so
// the panel doesn't reshuffle its rows as the spine streams in.
const NODE_KIND_ORDER: NodeKind[] = [
  "section", "paragraph", "sentence", "clause", "quote", "reference",
  "term", "character", "place", "concept", "event", "object", "group",
];
const EDGE_KIND_ORDER: EdgeKind[] = [
  "part_of", "precedes", "mentions", "references", "quotes", "co_occurs_with",
  "interacts_with", "located_in", "relates_to", "causes", "similar_to",
];
const PROV_ORDER: Provenance[] = ["structural", "semantic", "embedding"];

function renderStats(nodes: GraphNode[], edges: GraphEdge[]): void {
  if (nodes.length === 0) {
    statsBodyEl.innerHTML = '<div class="stat-empty">analyzing…</div>';
    return;
  }

  // One pass each: node-kind tally; edge-kind + provenance tally; node degree
  // (incident edge count, used to rank the key terms). idOf handles links whose
  // endpoints graphData() has already resolved from id strings to node objects.
  const nodeKind = new Map<NodeKind, number>();
  for (const n of nodes) nodeKind.set(n.kind, (nodeKind.get(n.kind) ?? 0) + 1);
  const edgeKind = new Map<EdgeKind, number>();
  const edgeProv = new Map<Provenance, number>();
  const degree = new Map<string, number>();
  for (const e of edges) {
    edgeKind.set(e.kind, (edgeKind.get(e.kind) ?? 0) + 1);
    edgeProv.set(e.provenance, (edgeProv.get(e.provenance) ?? 0) + 1);
    const s = idOf(e.source);
    const t = idOf(e.target);
    degree.set(s, (degree.get(s) ?? 0) + 1);
    degree.set(t, (degree.get(t) ?? 0) + 1);
  }

  const count = (k: NodeKind): number => nodeKind.get(k) ?? 0;
  const ratio = (a: number, b: number): string => (b > 0 ? (a / b).toFixed(1) : "—");

  // Composition — node kinds present, each a count + a bar scaled to the biggest.
  const maxNode = Math.max(1, ...NODE_KIND_ORDER.map((k) => count(k)));
  let comp = "";
  for (const k of NODE_KIND_ORDER) {
    const c = count(k);
    if (c === 0) continue;
    const col = nodeColor(k);
    const pct = Math.round((c / maxNode) * 100);
    comp +=
      `<div class="stat-row">` +
      `<span class="stat-swatch" style="background:${col}"></span>` +
      `<span class="stat-name">${esc(k)}</span>` +
      `<span class="stat-bar"><span class="stat-bar-fill" style="width:${pct}%;background:${col}"></span></span>` +
      `<span class="stat-count">${c.toLocaleString()}</span>` +
      `</div>`;
  }

  // Connections — edge kinds present + a provenance split (deterministic spine
  // vs. LLM-semantic vs. embedding-similarity), so the inferred share reads at a
  // glance against the structural backbone.
  const maxEdge = Math.max(1, ...EDGE_KIND_ORDER.map((k) => edgeKind.get(k) ?? 0));
  let conn = "";
  for (const k of EDGE_KIND_ORDER) {
    const c = edgeKind.get(k) ?? 0;
    if (c === 0) continue;
    const col = edgeColor(k);
    const pct = Math.round((c / maxEdge) * 100);
    conn +=
      `<div class="stat-row">` +
      `<span class="stat-swatch" style="background:${col}"></span>` +
      `<span class="stat-name">${esc(k.replace(/_/g, " "))}</span>` +
      `<span class="stat-bar"><span class="stat-bar-fill" style="width:${pct}%;background:${col}"></span></span>` +
      `<span class="stat-count">${c.toLocaleString()}</span>` +
      `</div>`;
  }
  let prov = "";
  for (const p of PROV_ORDER) {
    const c = edgeProv.get(p) ?? 0;
    if (c === 0) continue;
    prov += `<span><span class="dot" style="background:${PROV_META[p].dot}"></span>${esc(p)} ${c.toLocaleString()}</span>`;
  }

  // Shape — derived ratios that characterize the prose. Words are summed from the
  // sentence text (the only word-bearing nodes), so they grow with the spine too.
  let words = 0;
  for (const n of nodes) {
    if (n.kind === "sentence" && n.text) {
      const w = n.text.trim();
      if (w) words += w.split(/\s+/).length;
    }
  }
  const sentences = count("sentence");
  const metrics: Array<[string, string]> = [
    ["Words", words > 0 ? words.toLocaleString() : "—"],
    ["Words / sentence", ratio(words, sentences)],
    ["Clauses / sentence", ratio(count("clause"), sentences)],
    ["Sentences / paragraph", ratio(sentences, count("paragraph"))],
    ["Edges / node", ratio(edges.length, nodes.length)],
  ];
  const shape = metrics
    .map(
      ([k, v]) =>
        `<div class="stat-metric"><span class="k">${esc(k)}</span><span class="v">${v}</span></div>`
    )
    .join("");

  // Key terms — the most-connected term nodes (the document's conceptual hubs),
  // clickable to select the node through the same channel as the sidebar text.
  const terms = nodes.filter((n) => n.kind === "term");
  terms.sort((a, b) => (degree.get(b.id) ?? 0) - (degree.get(a.id) ?? 0));
  let termsHtml = "";
  for (const t of terms.slice(0, 14)) {
    const d = degree.get(t.id) ?? 0;
    termsHtml += `<span class="stat-term" data-node-id="${esc(t.id)}">${esc(t.label)}<span class="deg">${d}</span></span>`;
  }

  let html = "";
  html += `<div class="stat-section"><div class="stat-h">Composition · ${nodes.length.toLocaleString()} nodes</div>${comp}</div>`;
  html += `<div class="stat-section"><div class="stat-h">Connections · ${edges.length.toLocaleString()} edges</div>${conn || '<div class="stat-empty">none yet</div>'}${prov ? `<div class="stat-prov">${prov}</div>` : ""}</div>`;
  html += `<div class="stat-section"><div class="stat-h">Shape</div>${shape}</div>`;
  if (termsHtml) {
    html += `<div class="stat-section"><div class="stat-h">Key terms</div><div class="stat-terms">${termsHtml}</div></div>`;
  }
  statsBodyEl.innerHTML = html;
}

// --- D3: layout — the 3D canvas is the middle column ------------------------
// It yields width on the left to the stats panel (#43) and on the right to the
// document sidebar (D3), and offsets its own left edge so it never renders under
// either docked panel. The playback chip rides the same left offset so it clears
// the stats panel when that panel is open.
const SIDEBAR_W = 360;
const LEFT_W = 300;
let sidebarOpen = true;
let statsOpen = true;

function layoutGraph(): void {
  const left = statsOpen ? LEFT_W : 0;
  const right = sidebarOpen ? SIDEBAR_W : 0;
  const w = window.innerWidth - left - right;
  container.style.left = `${left}px`;
  graph.width(Math.max(320, w)).height(window.innerHeight);
  playbackEl.style.left = `${left + 16}px`;
}

function setSidebar(open: boolean): void {
  sidebarOpen = open;
  document.body.classList.toggle("sidebar-collapsed", !open);
  layoutGraph();
  graph.zoomToFit(600, 80);
}

function setStats(open: boolean): void {
  statsOpen = open;
  document.body.classList.toggle("stats-collapsed", !open);
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

// Engagement status (#39): while the structural spine is on screen but the slow,
// CPU-bound model-backed extraction is still running in the background, surface a
// gently pulsing "weaving…" line so the wait reads as active work, not a hang.
function setSemanticPending(on: boolean, label = "weaving semantic layer…"): void {
  semanticStatusEl.classList.toggle("hidden", !on);
  semanticStatusEl.innerHTML = on ? `<span class="pulse">✦</span> ${esc(label)}` : "";
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

// Stats panel collapse/reopen (mirrors the sidebar). Clicking a key-term chip
// selects that node through the same channel as the sidebar text + details panel.
statsCollapseEl.addEventListener("click", () => setStats(false));
statsReopenEl.addEventListener("click", () => setStats(true));
statsBodyEl.addEventListener("click", (e) => {
  const el = (e.target as HTMLElement).closest("[data-node-id]");
  if (el) selectNodeById(el.getAttribute("data-node-id")!);
});

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

// --- Phase 5 #1: clear-on-open + live elapsed timer -------------------------
// A slow desktop semantic walk runs tens of seconds to minutes on a large doc
// (the user's 650 KB novel). Without feedback the wait reads as a hang, so a
// readout in the left rail ticks from the instant a document is opened, through
// walk → build → semantic-weave, then freezes at the total. It starts in
// loadAndBuild *before* the await on the walk (so it counts the walk itself) and
// stops on the structural `done`, or — when a slow model-backed delta is in
// flight — in that delta's `.finally` (the genuinely slow CPU phase). A 100 ms
// tick is smooth enough to read as live without burdening the render loop.
let elapsedStart = 0;
let elapsedTimer: number | null = null;

function fmtElapsed(ms: number): string {
  const s = ms / 1000;
  if (s < 60) return `${s.toFixed(1)}s`;
  const m = Math.floor(s / 60);
  const rem = Math.round(s % 60);
  return `${m}m ${rem.toString().padStart(2, "0")}s`;
}

function startElapsed(): void {
  elapsedStart = performance.now();
  if (elapsedTimer !== null) window.clearInterval(elapsedTimer);
  elapsedEl.classList.add("running");
  const tick = (): void => {
    elapsedEl.textContent = `⏱ ${fmtElapsed(performance.now() - elapsedStart)}`;
  };
  tick();
  elapsedTimer = window.setInterval(tick, 100);
}

function stopElapsed(): void {
  if (elapsedTimer !== null) {
    window.clearInterval(elapsedTimer);
    elapsedTimer = null;
  }
  elapsedEl.classList.remove("running");
  // Freeze at the final total (one last read so the displayed value is exact).
  elapsedEl.textContent = `⏱ ${fmtElapsed(performance.now() - elapsedStart)}`;
}

// Tear down any running build and stream a fresh one from `source`. Everything
// that differs per-document (counts, sequence, dev handles) flows from here, so
// the upload and drag-drop paths converge on this one function.
function startBuild(source: BuildSource): void {
  // When a slow model-backed delta is pending, the elapsed timer keeps running
  // past the structural `done` and stops only when that delta settles (below);
  // on the pure structural path it stops on `done`.
  const hasPendingDelta = Boolean(source.pendingDelta);
  player?.pause();
  cancelPendingApply?.(); // drop any trailing apply still queued from the old build
  cancelPendingApply = null;
  clearSearch();
  graph.graphData({ nodes: [], links: [] });
  sidebarDocEl.innerHTML = '<div class="doc-empty">building…</div>';
  statsBodyEl.innerHTML = '<div class="stat-empty">analyzing…</div>';
  statsEl.textContent = `${source.nodeCount} nodes · ${source.edgeCount} edges (${source.origin})`;
  renderRouting(source); // B5 — surface the class + resolved pipeline / downgrade
  setSemanticPending(false); // cleared now; turned on below if a delta is pending
  applyForceTuning(source.nodeCount); // size the force field for the (final) spine
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
    // Layers mode pins Y by structural depth; nodes just streamed in arrive
    // unpinned, so re-pin the live set each commit to keep the strata crisp.
    if (currentLayout === "layers") pinLayers(graph.graphData().nodes);
    renderSidebar(nodes); // D3 — unfold the document in step with the graph
    renderStats(nodes, edges); // #43 — refresh the left analysis panel in step
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
        // Pure structural path: this is the total. (When a model-backed delta is
        // pending the timer keeps running and freezes in that delta's .finally.)
        if (!hasPendingDelta) stopElapsed();
      } else if (now - lastFitAt > 250) {
        graph.zoomToFit(500, 80);
        lastFitAt = now;
      }
    },
  });
  player = p;
  p.play();

  // #39 — engagement: the structural spine is streaming in above. If a slow,
  // model-backed semantic/embedding build is running in the background, weave its
  // delta onto the live graph the instant it finishes, and keep the user posted
  // (CPU inference can take tens of seconds) instead of leaving a static spine.
  if (source.pendingDelta) {
    const weaving =
      source.routing?.resolvedPipeline === "embedded_build"
        ? "computing similarity edges…"
        : "weaving semantic layer…";
    setSemanticPending(true, weaving);
    source.pendingDelta
      .then((delta) => {
        if (player !== p) return; // a newer build replaced us mid-extraction
        if (delta.fellBack) {
          source.fellBack = true;
          renderRouting(source); // now show "no model on disk; using the spine"
        } else if (delta.events.length > 0) {
          p.append(delta.events);
          source.nodeCount += delta.nodeCount;
          source.edgeCount += delta.edgeCount;
          statsEl.textContent = `${source.nodeCount} nodes · ${source.edgeCount} edges (${source.origin})`;
          applyForceTuning(source.nodeCount); // re-size the field for the grown graph
        }
      })
      .catch((err) => console.error("semantic delta failed:", err))
      .finally(() => {
        // Only freeze the timer if we're still the live build — a newer document
        // opened mid-extraction owns the timer now and must keep ticking.
        if (player === p) {
          setSemanticPending(false);
          stopElapsed(); // the slow CPU phase is done — this is the real total
        }
      });
  }

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
  // Phase 5 #1 — clear the previous document the *instant* a new one is opened,
  // and start the elapsed timer here (before the await) so it counts the walk
  // itself. Otherwise the old graph lingers through a slow desktop walk with no
  // sign of progress, reading as a hang. startBuild() re-clears after the walk
  // resolves; this pre-clear is what makes the old doc vanish immediately.
  startElapsed();
  player?.pause();
  cancelPendingApply?.(); // a late trailing apply must not write into the cleared scene
  cancelPendingApply = null;
  clearSearch();
  hideDetails();
  clearDocActive();
  setSemanticPending(false);
  graph.graphData({ nodes: [], links: [] });
  sidebarDocEl.innerHTML = '<div class="doc-empty">walking…</div>';
  statsBodyEl.innerHTML = '<div class="stat-empty">walking…</div>';
  statsEl.textContent = label ? `walking ${label}…` : "walking…";
  currentText = text ?? defaultDocument; // remember it for the meaning search
  try {
    const source = await loadBuildSource(text);
    startBuild(source);
  } catch (err) {
    console.error("walk failed:", err);
    statsEl.textContent = "walk failed — see console";
    stopElapsed(); // freeze the timer so it doesn't tick forever on a failed walk
  }
}

// The set the file picker's `accept` filter allows: plain text, markdown, and
// (new in #3) PDF. The drag-drop path bypasses `accept`, so it validates against
// this before ingest — dropping a binary we can't read (e.g. a .docx or an image)
// would otherwise be fed to readAsText and graph as mojibake.
function isIngestible(file: File): boolean {
  return (
    isPdf(file) ||
    file.type === "text/plain" ||
    file.type === "text/markdown" ||
    /\.(txt|md|markdown)$/i.test(file.name)
  );
}

// Read a dropped/selected file and rebuild from it. Plain text / markdown go
// through a naive readAsText; PDFs (#3 / ADR-0006) are routed through pdf.js in
// the frontend — its heavy library loads lazily, only on the first PDF open — so
// the same extraction serves the desktop and browser-WASM paths and the Rust
// default build stays native-free.
function ingestFile(file: File): void {
  if (isPdf(file)) {
    // Show progress immediately: clear the old doc + start the timer before the
    // (potentially slow) extract, reusing loadAndBuild's pre-await clear via a
    // dedicated "extracting…" beat so a big PDF doesn't read as a hang.
    statsEl.textContent = `extracting ${file.name}…`;
    extractPdfText(file)
      .then((text) => {
        if (text.trim().length === 0) {
          // Scanned / image-only PDFs yield no text — there's no OCR (a
          // documented backlog limit), so say so rather than graph an empty doc.
          statsEl.textContent = `${file.name}: no extractable text (scanned PDF?)`;
          return;
        }
        void loadAndBuild(text, file.name);
      })
      .catch((err) => {
        console.error("PDF extraction failed:", err);
        statsEl.textContent = `could not read ${file.name}`;
      });
    return;
  }
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
  if (!file) return;
  if (!isIngestible(file)) {
    statsEl.textContent = `unsupported file: ${file.name} — drop a .txt, .md, or .pdf`;
    return;
  }
  ingestFile(file);
});

void loadAndBuild();
