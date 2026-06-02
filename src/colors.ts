import type { NodeKind, EdgeKind } from "./types";

// Color-by-type palette. Structural spine kinds are cool/blue (the deterministic
// skeleton); semantic overlay kinds are warm/saturated (the LLM-inferred meaning)
// — so "color-by-provenance" reads at a glance against the fixture.
const NODE_COLORS: Record<NodeKind, string> = {
  // structural — blues/teals/greys
  section: "#7aa2ff",
  paragraph: "#6d8fd6",
  sentence: "#5f7fbf",
  clause: "#5d93a8",
  quote: "#52b6c4",
  reference: "#8f9bb3",
  term: "#9bd1e0",
  // semantic — warm, distinct hues
  character: "#ff6b6b",
  place: "#ffb454",
  concept: "#c792ea",
  event: "#ffd166",
  object: "#06d6a0",
  group: "#f78fb3",
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

// Structural edges are faint (the scaffold); semantic edges are brighter.
export function edgeColor(kind: EdgeKind): string {
  switch (kind) {
    case "part_of":
    case "precedes":
      return "#33415c";
    case "mentions":
    case "references":
    case "quotes":
    case "co_occurs_with":
      return "#3d5a80";
    default:
      return "#98c1d9";
  }
}
