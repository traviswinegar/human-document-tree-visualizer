// Mirrors the Rust graph schema in crates/doctree-core/src/schema.rs. The JSON
// produced by the walker (and the LLM semantic layer) deserializes into this.
// Keep the string unions in sync with NodeKind/EdgeKind/Provenance.

export type Provenance = "structural" | "semantic" | "embedding";

export type NodeKind =
  // structural spine
  | "section"
  | "paragraph"
  | "sentence"
  | "clause"
  | "quote"
  | "reference"
  | "term"
  // semantic overlay
  | "character"
  | "place"
  | "concept"
  | "event"
  | "object"
  | "group";

export type EdgeKind =
  | "part_of"
  | "precedes"
  | "mentions"
  | "references"
  | "quotes"
  | "co_occurs_with"
  | "interacts_with"
  | "located_in"
  | "relates_to"
  | "causes"
  | "similar_to";

export interface Span {
  start: number;
  end: number;
}

export interface GraphNode {
  id: string;
  kind: NodeKind;
  label: string;
  text?: string;
  span?: Span;
  provenance: Provenance;
}

export interface GraphEdge {
  id?: string;
  source: string;
  target: string;
  kind: EdgeKind;
  label?: string;
  weight?: number;
  provenance: Provenance;
}

export interface DocGraph {
  nodes: GraphNode[];
  edges: GraphEdge[];
}
