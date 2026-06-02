// Semantic-community clustering (Phase 7 #2 / ADR-00012).
//
// The force layout already *surfaces* community structure — on a novel, the dense
// per-chapter webs ball up and the sparse cross-links can't pull the balls
// together, so you see distinct blobs. But nothing holds that separation: it's a
// minimum-energy accident the growing semantic layer erodes. This module turns it
// into a designed feature.
//
//   detectCommunities() — run Louvain over ALL edges (incl. the LLM semantic
//   cross-links) to label each node with a community id. Seeded so a given graph
//   partitions the same way every run. A tested library is used deliberately: with
//   no JS test runner (ADR-0009 / ADR-00011) a hand-rolled Louvain couldn't be
//   unit-verified, so shipping it would be blind.
//
//   clusterForce() — a custom d3 force that, each tick, nudges every node toward
//   the running centroid of its community. The communities tighten in place; the
//   existing charge repulsion supplies the separation between the dense blobs (the
//   standard forceCluster recipe). Pure visualization — it never touches the
//   extracted graph (the truth), only how it's laid out.
import Graph from "graphology";
import louvain from "graphology-communities-louvain";

// Minimal edge shape detectCommunities needs (endpoints already resolved to ids).
export interface ClusterEdge {
  source: string;
  target: string;
  weight?: number;
}

// A d3-force node: 3d-force-graph mutates these (x/y/z + vx/vy/vz) in place each
// tick. In the 2D map z/vz are absent — the force guards for that.
interface SimNode {
  id?: string | number;
  x?: number;
  y?: number;
  z?: number;
  vx?: number;
  vy?: number;
  vz?: number;
}

// A d3-force: callable with the current alpha, plus initialize() to receive the
// live node array whenever the simulation's node set changes.
export interface ClusterForce {
  (alpha: number): void;
  initialize(nodes: SimNode[]): void;
}

// Deterministic PRNG (mulberry32). Passed to Louvain so a fixed graph yields a
// fixed partition run-to-run; a *grown* graph legitimately re-clusters.
function mulberry32(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

// Detect communities over the whole graph (structural spine + semantic overlay).
// Parallel edges are summed into a single weighted undirected edge; self-loops are
// dropped. Returns nodeId → communityId. Empty when there are no edges (the force
// then stays inert until real structure exists).
export function detectCommunities(
  nodeIds: readonly string[],
  edges: readonly ClusterEdge[],
): Map<string, number> {
  const result = new Map<string, number>();
  if (nodeIds.length === 0) return result;

  const g = new Graph({ type: "undirected", multi: false });
  for (const id of nodeIds) if (!g.hasNode(id)) g.addNode(id);
  for (const e of edges) {
    if (e.source === e.target) continue; // skip self-loops
    if (!g.hasNode(e.source)) g.addNode(e.source);
    if (!g.hasNode(e.target)) g.addNode(e.target);
    const w = e.weight ?? 1;
    if (g.hasEdge(e.source, e.target)) {
      g.updateEdgeAttribute(e.source, e.target, "weight", (prev) => ((prev as number) ?? 0) + w);
    } else {
      g.addEdge(e.source, e.target, { weight: w });
    }
  }

  // No edges → nothing to cluster meaningfully; leave the map empty.
  if (g.size === 0) return result;

  const mapping = louvain(g, {
    getEdgeWeight: "weight",
    resolution: 1,
    rng: mulberry32(0x5eed1234),
  });
  for (const id in mapping) result.set(id, mapping[id]);
  return result;
}

// Build the clustering force. `communityOf` resolves a node id to its community id
// (undefined → not yet labelled, node is skipped); `getStrength` is read live each
// tick so the toolbar slider can retune without re-registering the force. The pull
// is scaled by alpha so it anneals with the rest of the simulation, and is inert
// at strength ≤ 0 (slider at 0 = off). Dimension-safe: z is only touched when the
// nodes carry it (the 3D layouts), so the 2D map is unaffected; and it never fights
// the Layers-mode fy pins, since d3 zeroes velocity on fixed axes.
export function clusterForce(
  communityOf: (id: string) => number | undefined,
  getStrength: () => number,
): ClusterForce {
  let nodes: SimNode[] = [];

  const force = ((alpha: number): void => {
    const strength = getStrength();
    if (strength <= 0 || nodes.length === 0) return;

    const sumX = new Map<number, number>();
    const sumY = new Map<number, number>();
    const sumZ = new Map<number, number>();
    const count = new Map<number, number>();
    let has3d = false;

    for (const nd of nodes) {
      if (nd.id === undefined) continue;
      const c = communityOf(String(nd.id));
      if (c === undefined) continue;
      sumX.set(c, (sumX.get(c) ?? 0) + (nd.x ?? 0));
      sumY.set(c, (sumY.get(c) ?? 0) + (nd.y ?? 0));
      if (nd.z !== undefined) {
        sumZ.set(c, (sumZ.get(c) ?? 0) + nd.z);
        has3d = true;
      }
      count.set(c, (count.get(c) ?? 0) + 1);
    }

    const k = strength * alpha;
    for (const nd of nodes) {
      if (nd.id === undefined) continue;
      const c = communityOf(String(nd.id));
      if (c === undefined) continue;
      const n = count.get(c);
      if (!n) continue;
      nd.vx = (nd.vx ?? 0) + ((sumX.get(c) ?? 0) / n - (nd.x ?? 0)) * k;
      nd.vy = (nd.vy ?? 0) + ((sumY.get(c) ?? 0) / n - (nd.y ?? 0)) * k;
      if (has3d && nd.z !== undefined) {
        nd.vz = (nd.vz ?? 0) + ((sumZ.get(c) ?? 0) / n - nd.z) * k;
      }
    }
  }) as ClusterForce;

  force.initialize = (n: SimNode[]): void => {
    nodes = n;
  };

  return force;
}
