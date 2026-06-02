import type { NodeKind, EdgeKind } from "./types";

// Color-by-type palette. Two temperature families keep "color-by-provenance"
// readable at a glance: the deterministic spine spans the *cool* arc of the wheel
// (violet → indigo → azure → cyan → teal), the LLM-inferred semantic overlay the
// *warm* arc (red → orange → gold) plus a few vivid outliers. Every hue is high
// in lightness + saturation so nodes glow against the near-black background and
// stay distinct in a dense graph — the old palette was all muddy navy, which
// vanished as the graph grew (and the browser/WASM walk shows *only* the
// structural kinds, so those have to carry the whole picture on their own).
// These bright base colors also feed the bloom pass in main.ts: a brighter sphere
// clears the bloom threshold and picks up the glow.
const NODE_COLORS: Record<NodeKind, string> = {
  // structural — the cool arc, bright + spread so the spine reads colorful
  section: "#4f9dff", // azure — big anchor nodes
  paragraph: "#22d3ee", // cyan
  sentence: "#38bdf8", // sky — the dominant kind; luminous so the cloud isn't navy mud
  clause: "#818cf8", // indigo
  quote: "#2dd4bf", // teal
  reference: "#9fb3d9", // light slate — meta, intentionally the quietest
  term: "#a78bfa", // violet
  // semantic — the warm arc + vivid outliers, each clearly distinct
  character: "#ff5d5d", // red
  place: "#ffa53c", // orange
  concept: "#d946ef", // fuchsia
  event: "#ffd93d", // gold
  object: "#4ade80", // lime green
  group: "#f472b6", // pink
};

const NODE_SIZE: Record<NodeKind, number> = {
  section: 8,
  paragraph: 5,
  sentence: 3,
  clause: 2.5,
  quote: 3,
  reference: 2.5,
  term: 3,
  character: 6,
  place: 6,
  concept: 6,
  event: 5,
  object: 5,
  group: 6,
};

export function nodeColor(kind: NodeKind): string {
  return NODE_COLORS[kind] ?? "#aaaaaa";
}

export function nodeSize(kind: NodeKind): number {
  return NODE_SIZE[kind] ?? 3;
}

// Edges stay subordinate to the (glowing) nodes — the scaffold, not the subject —
// but the old slate was so dark it dissolved into the background. Lifted just
// enough to trace the structure: the hierarchy/order spine is a cool slate-blue,
// semantic links a brighter periwinkle, anything else a pale steel.
export function edgeColor(kind: EdgeKind): string {
  switch (kind) {
    case "part_of":
    case "precedes":
      return "#4a6391"; // structural spine — visible but quiet
    case "mentions":
    case "references":
    case "quotes":
    case "co_occurs_with":
      return "#6d8fd6"; // semantic links — brighter
    default:
      return "#acd0e8";
  }
}
