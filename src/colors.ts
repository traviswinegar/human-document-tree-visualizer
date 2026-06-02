import type { NodeKind, EdgeKind } from "./types";

// Color-by-type palette. Two temperature families keep "color-by-provenance"
// readable at a glance: the deterministic spine spans the *cool* arc of the wheel
// (violet → indigo → blue → sky → cyan → teal), the LLM-inferred semantic overlay
// the *warm* arc (red → orange → gold) plus a few vivid outliers. These are
// *robust*, fully-saturated jewel tones (Tailwind 500/600 level), not the earlier
// pastel/luminous set — they hold their identity on their own, so the graph reads
// as bold color even with the bloom dialed almost off. Each hue is well-separated
// from its neighbours so a dense cloud stays legible (and the browser/WASM walk
// shows *only* the structural kinds, so those seven have to carry the picture).
const NODE_COLORS: Record<NodeKind, string> = {
  // structural — the cool arc, deep + saturated, spread across the cool wheel
  term: "#8b5cf6", // violet
  clause: "#6366f1", // indigo
  section: "#2563eb", // bold blue — big anchor nodes
  sentence: "#0ea5e9", // sky — the dominant kind
  paragraph: "#06b6d4", // cyan
  quote: "#14b8a6", // teal
  reference: "#64748b", // slate — meta, intentionally the quietest
  // semantic — the warm arc + vivid outliers, each clearly distinct
  character: "#ef4444", // red
  place: "#f97316", // orange
  event: "#f59e0b", // amber/gold
  concept: "#d946ef", // fuchsia
  object: "#22c55e", // green
  group: "#ec4899", // rose/pink
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
