# ADR-0009 — Hierarchical edge bundling as a single merged geometry

- **Status:** Accepted (2026-06-02)
- **Phase:** 6 (#4)
- **Supersedes / superseded by:** refines the Phase 5 #2 "first cut" (kept; not an ADR)

## Context

Phase 5 #2 gave cross-links a "first cut" at bundling: each edge was bowed with
3d-force-graph's built-in `linkCurvature` + `linkCurveRotation`, keyed by the
structural *region* of its endpoints (the nearest section/paragraph ancestor,
hashed to a stable angle). Co-routed edges fanned into loose trails — it read
better than a straight hairball, but two problems remained:

1. **Every edge was still its own scene object.** A document with thousands of
   semantic/similarity cross-links meant thousands of curved line meshes — a draw
   call each. On large graphs the cross-link layer is the framerate cost.
2. **The arc ignored the actual hierarchy.** "Region" was a single ancestor
   bucket; the bow was a fixed quadratic that didn't follow the containment tree.
   Edges sharing a *path* up the tree didn't physically overlap, so the bundling
   was cosmetic rather than structural.

The product is a *hierarchy* (the `part_of` containment spine the walker builds).
Real **hierarchical edge bundling (HEB)** routes a cross-link A→B *through* that
hierarchy — up A's ancestors to the lowest common ancestor (LCA) of A and B, then
back down to B — so edges that share a path up the tree coincide and read as one
thick bundle that frays only where the hierarchy diverges.

Constraints framing the choice:
- **ADR-0001 decoupling:** pure frontend geometry; the Rust workspace is untouched
  and the default build stays native-free. Serves both render paths identically.
- **Live layout:** the force sim already places every node, including the
  section/paragraph ancestors. The bundle must track *those live positions*, not a
  separate assumed radial tree, or it would drift from the graph it annotates.

## Decision

A new portable module `src/bundling.ts` owns an **`EdgeBundle`** class that routes
every non-backbone edge through its LCA path in the `part_of` tree and samples all
of them into **one** `THREE.LineSegments` `BufferGeometry` (per-vertex colour).
Thousands of cross-links cost **one draw call**.

- **Control path = hierarchy path.** `controlPath(s, t)` climbs `part_of` from
  each endpoint (ancestry capped at 64 hops against malformed/cyclic containment),
  intersects the two ancestor chains to find the LCA, and returns
  `s → … → LCA → … → t` as node ids. The *node positions* the force sim resolved
  are the spline control points, so the bundle tracks the live layout.
- **Smooth strands.** A path of ≥3 control points is sampled with a centripetal
  `CatmullRomCurve3` (hugs the control points without overshoot, so strands stay
  tucked against the hierarchy); divisions scale with path length
  (`min(24, max(4, (pts-1)*4))`), capped for memory. A 2-point path is a straight
  segment.
- **Backbone stays native & straight.** `part_of` / `precedes` is the spine you
  read along; `isBackboneKind` excludes it from the bundle. While the bundle is
  shown, `main.ts` hides 3d-force-graph's own cross-links via
  `.linkVisibility((l) => !bundleEdges || !bundleShown || isBackboneKind(l.kind))`
  so the native and merged renderers never double-draw.
- **Rebuild on settle, drop on motion.** The geometry is built from settled
  positions; `onEngineStop` rebuilds it and `onEngineTick` clears it the instant
  the sim reheats (a stale bundle would lag the moving nodes). Data-change sites
  (build complete, semantic/similarity delta woven in, saved-graph restore) call
  `rebuildMergedBundle()` so the bundle reflects new edges; the engine handlers
  manage the moving↔settled transitions.

`EdgeBundle` is pure geometry — no DOM, no graph library — so it is testable and
portable in isolation.

## Alternatives considered

1. **Keep the first cut (per-edge `linkCurvature`).** Zero new code.
   **Rejected:** one draw call per edge, and the bow never followed the hierarchy
   — the two problems above are exactly what #4 set out to fix.
2. **Per-edge curve meshes routed through the LCA (TubeGeometry / Line2 each).**
   Gets the hierarchy routing but keeps the thousands-of-objects cost — the more
   expensive half of the problem. **Rejected.**
3. **GPU instanced curves / a custom shader bundle.** Best raw throughput.
   **Rejected (for now):** large complexity + maintenance surface for a frontend
   that has no shader infrastructure yet; one merged `LineSegments` already
   collapses the draw calls to one and is plenty for the target document sizes.
4. **Force-directed edge bundling (FDEB, Holten & van Wijk).** Bundles by mutual
   edge attraction, ignoring any tree. **Rejected:** we *have* a real hierarchy;
   routing through it is cheaper and more meaningful than simulating attraction.

## Consequences

- (+) One draw call for the entire cross-link layer regardless of edge count.
- (+) Bundling is structural: edges sharing an ancestor path physically overlap,
  so the picture reflects the document's containment, not a cosmetic hash.
- (+) Tracks the live force layout (control points are the sim's node positions).
- (+) Pure frontend geometry; Rust untouched, both render paths identical.
- (−) The bundle is rebuilt (not incrementally updated) on settle — O(edges ×
  divisions); acceptable because it runs once per settle, not per frame.
- (−) While the sim is hot the bundle is hidden (native links show through that
  window) — an intentional trade so the geometry never lags moving nodes.
- (−) Cross-tree edges (no shared ancestor) route through both roots; they still
  bundle by shared region root but fray earlier than same-tree edges (documented).

## Invariant (pinned)

The frontend has no JS test runner (every frontend unit is verified by the
standing gate), so the pinned check is:

- **Gate:** `npx tsc --noEmit` (exit 0) + `npm run build` (exit 0, no new errors).
- **Behavioral pin:** `src/bundling.ts` — `isBackboneKind` keeps `part_of` /
  `precedes` out of the bundle; `EdgeBundle.rebuild` routes every other edge
  through its LCA path and emits a single `LineSegments`. `main.ts` swaps native
  cross-links for the merged geometry via `linkVisibility` while `bundleShown`,
  rebuilds on `onEngineStop`, and clears on `onEngineTick`. If bundling regresses,
  the merged-geometry build or the visibility swap is where it shows.
