// Phase 6 #4 (ADR-0009) — hierarchical edge bundling as merged line geometry.
//
// The Phase 5 #2 "first cut" bowed each cross-link with 3d-force-graph's built-in
// linkCurvature: one quadratic arc per edge, keyed by region so co-routed edges
// fanned into trails. It read better than a straight hairball, but every edge was
// still its own scene object and the arc ignored the document's actual hierarchy.
//
// This is the real thing: **hierarchical edge bundling (HEB)**. A cross-link from
// node A to node B is routed *through the containment tree* — up A's `part_of`
// ancestors to the lowest common ancestor (LCA) of A and B, then back down to B —
// so edges that share a path up the tree physically overlap and read as one thick
// bundle that only frays where the hierarchy diverges. The ancestor *node
// positions* (which the force sim already places) are the spline control points,
// so the bundling tracks the live layout instead of assuming a fixed radial tree.
//
// Crucially it is **one** THREE object: all bundled edges are sampled into a
// single BufferGeometry (a LineSegments with per-vertex colour), so thousands of
// cross-links cost one draw call instead of thousands of curve meshes. The caller
// hides 3d-force-graph's own cross-links while the bundle is shown and rebuilds
// the geometry when the layout settles (positions stop moving). Pure geometry —
// no DOM, no graph library — so this module is portable and testable in isolation.

import {
  BufferGeometry,
  Float32BufferAttribute,
  LineBasicMaterial,
  LineSegments,
  CatmullRomCurve3,
  Vector3,
  Color,
  type Scene,
} from "three";

// A node with the position the force sim resolved onto it (x/y/z default to 0
// before the first tick, which is harmless — the edge just routes through origin).
export interface BundleNode {
  id: string;
  kind: string;
  x?: number;
  y?: number;
  z?: number;
}

// A resolved edge: endpoint ids + kind (the backbone — part_of / precedes — is
// never bundled; it's the spine you read along and stays straight & native).
export interface BundleEdge {
  sourceId: string;
  targetId: string;
  kind: string;
}

export interface BundleOptions {
  /** Containment depth per node kind (section 0 … entity 5). Drives the tree we
   *  climb to find LCAs, exactly as the Layers view and the first cut did. */
  kindLayer: Record<string, number>;
  /** Deepest layer, used as the fallback for unknown kinds. */
  maxLayer: number;
  /** Edge kind → hex colour (the same palette the native links use). */
  colorOf: (kind: string) => string;
  /** Line opacity (matches the native linkOpacity so the swap is seamless). */
  opacity?: number;
}

const BACKBONE = new Set(["part_of", "precedes"]);

/** Is this the deterministic spine (never bundled)? */
export function isBackboneKind(kind: string): boolean {
  return BACKBONE.has(kind);
}

// Build child→parent from the `part_of` edges: containment points deep→shallow,
// so the shallower-layer endpoint is the parent. This is the tree HEB climbs.
function buildParentMap(
  edges: readonly BundleEdge[],
  layerOf: (id: string) => number
): Map<string, string> {
  const parent = new Map<string, string>();
  for (const e of edges) {
    if (e.kind !== "part_of") continue;
    const ls = layerOf(e.sourceId);
    const lt = layerOf(e.targetId);
    if (ls >= lt) parent.set(e.sourceId, e.targetId);
    else parent.set(e.targetId, e.sourceId);
  }
  return parent;
}

// Ancestors of `id` from itself up to a root, capped so a cyclic/malformed
// containment chain can never loop forever.
function ancestry(id: string, parent: Map<string, string>): string[] {
  const chain = [id];
  let cur = id;
  for (let hops = 0; hops < 64; hops++) {
    const up = parent.get(cur);
    if (up === undefined || up === cur) break;
    chain.push(up);
    cur = up;
  }
  return chain;
}

// The HEB control path for an edge: source → … → LCA → … → target, as node ids.
// If the two endpoints share no ancestor (different trees) the path runs through
// both roots, which still bundles edges that share a region root. Returns just
// the two endpoints when there's nothing to route through (a straight segment).
function controlPath(
  sourceId: string,
  targetId: string,
  parent: Map<string, string>
): string[] {
  const up = ancestry(sourceId, parent); // [s, …, sRoot]
  const down = ancestry(targetId, parent); // [t, …, tRoot]
  const downIndex = new Map<string, number>();
  down.forEach((id, i) => downIndex.set(id, i));

  // First ancestor of source that is also an ancestor of target = LCA.
  let lcaUp = up.length - 1; // default: source root (no common ancestor case)
  let lcaDown = down.length - 1;
  for (let i = 0; i < up.length; i++) {
    const j = downIndex.get(up[i]);
    if (j !== undefined) {
      lcaUp = i;
      lcaDown = j;
      break;
    }
  }
  // s → … → LCA (inclusive), then LCA's child on the target side → … → t.
  const path = up.slice(0, lcaUp + 1);
  for (let j = lcaDown - 1; j >= 0; j--) path.push(down[j]);
  return path;
}

/** Owns the single merged LineSegments object and its (re)build. Construct once,
 *  attach to the graph's THREE scene, then rebuild() on settle / clear() on
 *  toggle-off. */
export class EdgeBundle {
  private scene: Scene;
  private opts: Required<Pick<BundleOptions, "opacity">> & BundleOptions;
  private geometry: BufferGeometry | null = null;
  private mesh: LineSegments | null = null;
  private material: LineBasicMaterial;

  constructor(scene: Scene, opts: BundleOptions) {
    this.scene = scene;
    this.opts = { opacity: 0.4, ...opts };
    this.material = new LineBasicMaterial({
      vertexColors: true,
      transparent: true,
      opacity: this.opts.opacity,
      depthWrite: false, // let overlapping bundle strands blend instead of z-fighting
    });
  }

  /** Is the merged bundle currently built and in the scene? */
  get shown(): boolean {
    return this.mesh !== null;
  }

  // Sample one edge's control path into a smooth polyline and append its segments
  // (vertex pairs) + colour to the running buffers.
  private appendEdge(
    pts: Vector3[],
    color: Color,
    positions: number[],
    colors: number[]
  ): void {
    let line: Vector3[];
    if (pts.length <= 2) {
      line = pts; // straight: no curve to sample
    } else {
      // Centripetal Catmull-Rom hugs the control points without overshoot, so the
      // bundle stays tucked against the hierarchy path. Divisions scale with path
      // length (more ancestors → a longer, smoother arc), capped for memory.
      const divisions = Math.min(24, Math.max(4, (pts.length - 1) * 4));
      line = new CatmullRomCurve3(pts, false, "centripetal").getPoints(divisions);
    }
    for (let i = 0; i < line.length - 1; i++) {
      const a = line[i];
      const b = line[i + 1];
      positions.push(a.x, a.y, a.z, b.x, b.y, b.z);
      colors.push(color.r, color.g, color.b, color.r, color.g, color.b);
    }
  }

  /** Rebuild the merged geometry from the current node positions. Bundles every
   *  non-backbone edge through its LCA path; backbone edges are left to the native
   *  renderer. Replaces any previously built geometry. */
  rebuild(nodes: readonly BundleNode[], edges: readonly BundleEdge[]): void {
    const { kindLayer, maxLayer, colorOf } = this.opts;
    const layerById = new Map<string, number>();
    const posById = new Map<string, Vector3>();
    for (const n of nodes) {
      layerById.set(n.id, kindLayer[n.kind] ?? maxLayer);
      posById.set(n.id, new Vector3(n.x ?? 0, n.y ?? 0, n.z ?? 0));
    }
    const layerOf = (id: string): number => layerById.get(id) ?? maxLayer;
    const parent = buildParentMap(edges, layerOf);

    const positions: number[] = [];
    const colors: number[] = [];
    const colorCache = new Map<string, Color>();
    for (const e of edges) {
      if (isBackboneKind(e.kind)) continue;
      const path = controlPath(e.sourceId, e.targetId, parent);
      const pts: Vector3[] = [];
      for (const id of path) {
        const p = posById.get(id);
        if (p) pts.push(p);
      }
      if (pts.length < 2) continue; // endpoints missing → nothing to draw
      let color = colorCache.get(e.kind);
      if (!color) {
        color = new Color(colorOf(e.kind));
        colorCache.set(e.kind, color);
      }
      this.appendEdge(pts, color, positions, colors);
    }

    // Swap the geometry in place (dispose the old to free GPU buffers).
    this.geometry?.dispose();
    this.geometry = new BufferGeometry();
    this.geometry.setAttribute("position", new Float32BufferAttribute(positions, 3));
    this.geometry.setAttribute("color", new Float32BufferAttribute(colors, 3));
    if (!this.mesh) {
      this.mesh = new LineSegments(this.geometry, this.material);
      this.mesh.frustumCulled = false; // spans the whole graph; never cull it
      this.scene.add(this.mesh);
    } else {
      this.mesh.geometry = this.geometry;
    }
  }

  /** Remove the merged bundle from the scene and free its buffers. */
  clear(): void {
    if (this.mesh) {
      this.scene.remove(this.mesh);
      this.mesh = null;
    }
    this.geometry?.dispose();
    this.geometry = null;
  }
}
