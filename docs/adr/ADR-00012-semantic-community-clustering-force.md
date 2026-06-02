# ADR-00012 — Semantic-community clustering force

- **Status:** Accepted (2026-06-02)
- **Phase:** 7 (#2)
- **Deciders:** Travis (user), agent
- **Depends on:** [ADR-0001](ADR-0001-stack-tauri-rust-web3d.md),
  [ADR-0009](ADR-0009-hierarchical-edge-bundling-merged-geometry.md),
  [ADR-00011](ADR-00011-adaptive-large-graph-render-lod.md)
- **Supersedes / superseded by:** none

## Context

Watching a large doc build, the user noticed distinct **clusters form early** — on
a novel, three clean blobs — and asked how to *preserve* them. Those blobs are
real **community structure**: within one chapter every clause/sentence/term is
densely chained (`part_of` + `precedes`), while between chapters the only links
are sparse semantic ones (`similar_to`, `co_occurs_with`). The force sim relaxes
into a shape where each dense web balls up and the sparse cross-links can't pull
the balls together.

But nothing **holds** that separation — it's a minimum-energy accident, and it is
actively eroded by:

1. **The semantic layer keeps adding cross-cluster edges.** Every inferred
   `similar_to` / `co_occurs_with` between terms in different chapters is a spring
   pulling two blobs together; enough of them merge the blobs into one ball.
2. **`applyForceTuning` scales link distance up with size** (40 → 95), which
   spreads *all* connected nodes uniformly — loosening cohesion, not grouping.

So "loosen the edges to focus on clustering" (the user's first instinct) has the
sign backwards: looser links make each blob mushier and let repulsion smear
everything into one cloud. The durable fix is to stop relying on emergence and
**anchor each node to its cluster**.

When asked what defines a "cluster" to preserve, the user chose a **detected
semantic community** (over *all* edges, including the LLM cross-links) rather than
the structural `part_of` root — so the lens can surface a recurring character's
web that spans chapters, not merely the structural skeleton. For tightness the
user chose "you decide / tune live": the agent picks size-scaled defaults and
exposes the knob so we eyeball-tune it in the running app.

Constraints (unchanged):
- **ADR-0001 decoupling:** pure frontend; the Rust workspace and the native-free
  default build are untouched. Clustering is a *visualization* concern — it never
  changes the extracted graph (the truth), only how it is laid out.
- **No JS test runner** (ADR-0009 / ADR-00011): frontend units are verified by the
  standing gate (`tsc --noEmit` + `vite build`) plus the user's live end test.
- Detection must be fast at the 18 437-node / 46 477-edge scale and must **not**
  run per frame.

## Decision

Add `src/clustering.ts` plus a custom d3 force wired into `main.ts`.

1. **Detection — `detectCommunities(nodeIds, edges)`.** Build an *undirected,
   weighted* graphology graph over **all** edges including semantic (parallel
   edges summed; weight = `edge.weight ?? 1`) and run **Louvain**
   (`graphology-communities-louvain`) seeded with a fixed PRNG (`mulberry32`) so a
   given graph partitions the same way every run. Returns `Map<nodeId, communityId>`.
   Reusing a battle-tested implementation is deliberate: with no JS test runner a
   hand-rolled Louvain could not be unit-verified, so shipping it would be blind.

2. **Force — `clusterForce(...)`.** A custom d3 force registered as
   `graph.d3Force("cluster", …)`. Each tick it computes the running centroid of
   each community and nudges every node a fraction (`strength · alpha`) toward its
   own community centroid. Communities **tighten in place**; the inter-community
   *separation* is supplied by the existing charge repulsion now acting between
   dense blobs (the standard `forceCluster` recipe). The force is **dimension-safe**
   (guards `z` so the 2D map is unaffected), inert for nodes that have no community
   yet, and inert when `strength ≤ 0` (slider at 0 = off). It never fights the
   Layers-mode `fy` pins (d3 zeroes velocity on fixed axes).

3. **When detection runs — on settle, guarded, not per frame.** Recompute on
   `onEngineStop`, gated by a `nodes.length:links.length` key so it only runs when
   topology actually changed. That single hook covers every topology change —
   build end, semantic-delta grow, saved-graph restore — because each eventually
   settles. After a recompute, one `d3ReheatSimulation()` lets the force pull the
   blobs tight; the subsequent settle finds the same key and is a no-op, so it
   terminates.

4. **Live-tunable strength.** `applyClusterStrength(n)` sets a default scaled by
   `spread(n)` — gentle on small graphs, firmer on large ones where blobs merge —
   and is called alongside `applyForceTuning` / `applyRenderLOD` at every site where
   the node count is known. A `#cluster` range slider in `#controls` exposes the
   knob (0 = off). The first slider input sets `userSetCluster`, after which the
   size-default never overrides the user's choice — mirroring ADR-00011's
   `userToggledBundle` hand-back.

**Deferred (logged, not done):** recoloring nodes by community (would clobber the
kind-based palette and the highlight-dimming pass), and an optional inter-centroid
repulsion term for more active separation.

## Alternatives considered

1. **Structural `part_of`-root clustering.** Free, deterministic, no algorithm.
   **Rejected:** only reproduces the structural skeleton; the user explicitly chose
   the *semantic* lens so cross-chapter groupings can emerge.
2. **Label propagation.** Simpler, faster, no dependency. **Rejected:** lower-quality
   and unstable communities, and on this graph it tends to collapse everything via
   the `precedes` chain.
3. **Hand-rolled Louvain.** No new dependency. **Rejected:** un-unit-testable here
   (no JS runner); modularity-gain bugs would ship silently. Prefer the tested lib.
4. **Recompute every tick / every `graphData`.** **Rejected:** wasteful at 46k edges;
   the partition is topology-stable, so once per size is enough.
5. **Inter-centroid repulsion inside the force.** More active separation.
   **Deferred:** more knobs; charge already separates the tightened blobs well
   enough for a first cut we tune by eye.

## Consequences

- (+) The clusters the user saw become a **designed, stable** feature: they survive
  heavy semantic cross-linking instead of dissolving as the layer grows.
- (+) The semantic lens can reveal **cross-chapter** groupings (e.g. a character's
  web), which a structural cluster key could not.
- (+) A live slider lets us tune separation by eye; the size-scaled default means it
  behaves sensibly untouched.
- (+) Pure frontend; Rust / native default untouched (ADR-0001). Reuses the
  `onEngineStop` settle hook already present for edge bundling.
- (−) Two new runtime deps (`graphology`, `graphology-communities-louvain`).
- (−) Louvain re-partitions when topology changes; the seeded rng makes a *fixed*
  graph stable but a grown graph legitimately re-clusters (expected, not a bug).
- (−) No agent-runnable verification of the visual result (no JS runner) — gate +
  user eyeball only, per established frontend precedent.
- (−) Centroid attraction can over-compact a community at high strength; mitigated
  by the slider and a conservative size-scaled default.

## Invariant (pinned)

The frontend has no JS test runner (every frontend unit is verified by the
standing gate), so the pinned check is:

- **Gate:** `npx tsc --noEmit` (exit 0) + `npm run build` (exit 0, no new errors).
- **Behavioral pin:** `src/clustering.ts` exports `detectCommunities` (seeded,
  weighted Louvain over **all** edges) and `clusterForce` (centroid-attraction d3
  force, dimension-safe, inert at strength ≤ 0). `src/main.ts` registers
  `graph.d3Force("cluster", …)`, recomputes the community map on `onEngineStop`
  guarded by the `nodes:links` count key followed by one reheat, and
  `applyClusterStrength(n)` is called alongside `applyForceTuning` at every
  size-known site. The `#cluster` slider sets `userSetCluster` to hand control to
  the user (mirrors `userToggledBundle`). If clustering stops working or thrashes,
  these are where it shows.
- **Deferred lever (open Catch-all):** node recolor by community; optional
  inter-centroid repulsion term.

## Anchors

- `src/clustering.ts` — `detectCommunities`, `clusterForce`, the `mulberry32` seed.
- `src/main.ts` — `communityOf` map, `clusterStrength`, `userSetCluster`,
  `applyClusterStrength`, the `graph.d3Force("cluster", …)` registration, the
  `onEngineStop` recompute guard (`detectKey`), and the `#cluster` slider handler.
- `index.html` — the `#cluster` range input in `#controls`.
