# ADR-00011 — Adaptive render LOD for very large graphs

- **Status:** Accepted (2026-06-02)
- **Phase:** 7 (#1)
- **Deciders:** Travis (user), agent
- **Depends on:** [ADR-0001](ADR-0001-stack-tauri-rust-web3d.md),
  [ADR-0009](ADR-0009-hierarchical-edge-bundling-merged-geometry.md)
- **Supersedes / superseded by:** none

## Context

The app graphs a 650 KB / 400-page novel at **18 437 nodes / 46 477 edges**
successfully — the *build* lands. What the user flagged (non-blocking, "nice to
have") is that **orbit / zoom / pan interaction gets heavy** at that scale: once
the layout has settled and you're just navigating, the framerate sags.

After [ADR-0009](ADR-0009-hierarchical-edge-bundling-merged-geometry.md) (#61)
the cross-link layer is **one** draw call *when the merged bundle is shown*. But
on a fresh large graph the per-frame render cost is still dominated by:

1. **One draw call per node sphere.** 3d-force-graph renders each node as its own
   `THREE.Mesh`; ~18k nodes ⇒ ~18k draw calls every frame during orbit.
2. **Animated semantic particles.** `linkDirectionalParticles` puts moving
   particle meshes on every semantic edge — continuous per-frame geometry churn
   that scales with the semantic-edge count.
3. **Native cross-links when bundling is off.** Bundling is a manual toolbar
   toggle (#34/#61), **off by default**, so a large graph that the user hasn't
   manually bundled draws ~46k native line objects.

The merged bundle already exists and is tested; it was simply not engaged
automatically. The cheapest wins are therefore the ones that don't touch the
node-picking path at all.

Constraints (unchanged):
- **ADR-0001 decoupling:** pure frontend; the Rust workspace and the native-free
  default build are untouched.
- **No JS test runner** (per ADR-0009): frontend units are verified by the
  standing gate (`tsc --noEmit` + `vite build`) plus the user's live end test.
- **No agent-runnable interaction test:** the Tauri webview is not headlessly
  introspectable, so any change to *picking / hover / selection* cannot be
  verified by the agent — only by the user firing the app up.

## Decision

Add a single size-tiered **`applyRenderLOD(n)`** in `main.ts`, called everywhere
the final node count becomes known (the same sites as `applyForceTuning`: initial
baseline, build start, semantic-delta grow, saved-graph restore, layout switch).
Above `LARGE_GRAPH_NODES = 2000` it sheds the three cheapest-to-lose costs:

1. **Auto-enable the merged bundle** (ADR-0009's `setBundling(true)`): ~46k native
   cross-link draw calls → **1**. The backbone (`part_of` / `precedes`) stays
   native and straight, exactly as ADR-0009 specifies.
2. **Suppress semantic particles** (`linkDirectionalParticles(0)`): removes the
   continuous per-frame particle churn. The exact accessor (`SEMANTIC_PARTICLES`)
   is restored when the graph drops back below the threshold.
3. **Coarsen node geometry** (`nodeResolution(6)` vs the default `8`): fewer
   triangles per sphere — a vertex-shading win (it does **not** reduce draw
   calls; see deferred lever below).

Bundling is **auto-managed by size until the user takes manual control.** A
`userToggledBundle` flag is set the first time the toolbar toggle is clicked;
after that `applyRenderLOD` never forces bundling either way, so an explicit user
choice is never overridden. Until then, crossing the threshold turns bundling on
and dropping below it turns it back off.

**Separately (a dev-mode fix, not LOD):** node dragging is disabled
(`enableNodeDrag(false)`). Nothing in the app repositions a node by hand — the
force sim and the layout modes own every position — so the library's default-on
`DragControls` was pure dead weight, and its pointer-cancel path (reading a node
coord on an interrupted drag) was the source of the transient dev-mode
`Cannot read properties of undefined (reading 'x')` warning. Removing the unused
interaction removes the warning. Orbit/zoom/pan is the camera's `OrbitControls`,
a separate control, and is unaffected.

### Deferred lever: per-node InstancedMesh (the node draw calls)

The one remaining big lever is collapsing the ~18k node draw calls to **one** via
a `THREE.InstancedMesh`. It is **deliberately deferred**, not forgotten, because
it requires *replacing* 3d-force-graph's built-in node rendering, and with it the
library's **picking, hover labels, per-node colour/dimming, and size** — i.e.
re-implementing click selection by raycasting the `InstancedMesh` (`instanceId` →
node id), the click-vs-orbit pointer disambiguation OrbitControls currently owns,
the hover tooltip, per-instance colour for the highlight-dimming pass, and
per-instance scale for node size. That is a rewrite of the **core interaction
path**, and this project has **no agent-runnable verification** for interaction
(no JS test runner; the webview isn't headlessly introspectable). Shipping it
blind risks silently breaking selection on every graph. It is the right next step
*with interactive testing in the loop*, and is logged as the open Catch-all lever.

## Alternatives considered

1. **Do the full per-node InstancedMesh now.** Best raw win (node draw calls →
   one). **Rejected (for now):** un-verifiable picking/hover/selection rewrite of
   the core interaction path (see above); deferred to an interactive session.
2. **Importance-based node culling (hide deep/leaf nodes when zoomed out).**
   Cuts draw calls without touching picking. **Rejected:** it changes *what is
   shown* — a surprising, semantic change to the picture — where the LOD here only
   changes *how* the same graph is drawn.
3. **Leave bundling a manual toggle and just document "turn it on for big
   graphs".** Zero code. **Rejected:** the single biggest edge-side win was a
   click away and un-discovered by default; auto-engaging it by size is free
   (reuses tested machinery) and is what the user actually wants on a big graph.
4. **A fixed low `nodeResolution` for all graphs.** Simpler (no tier).
   **Rejected:** small graphs look better at the default resolution and pay no
   meaningful cost, so the coarsening is gated to where it helps.
5. **Disable particles globally.** **Rejected:** the flowing particles are the
   "this edge is LLM-inferred meaning" signal (ADR intent from B3); they're worth
   keeping on graphs small enough to afford them.

## Consequences

- (+) Large-graph navigation sheds the entire native cross-link layer (→ one
  bundled draw call), the per-frame particle churn, and a chunk of node vertex
  load — a multiplicative per-frame win with **zero** change to the picking path.
- (+) Reuses ADR-0009's tested bundle machinery; no new rendering subsystem.
- (+) Removes a real dev-mode warning by deleting an unused interaction
  (`enableNodeDrag(false)`).
- (+) Pure frontend; Rust untouched, default build still native-free (ADR-0001).
- (−) Node **draw calls** are unchanged (~one per node) — geometry is coarser but
  still one mesh each. The headline node-side lever (InstancedMesh) is deferred;
  on the very largest graphs orbit will still be node-draw-call-bound until then.
- (−) `LARGE_GRAPH_NODES = 2000` is a heuristic; a ~1800-node doc stays in the
  full-fidelity tier (particles on, bundling off) by design, but the boundary is
  a judgement call, not a measured knee.
- (−) Auto-managing bundling can surprise a user who expected the toggle to be
  the only control — mitigated by `userToggledBundle` handing control back the
  instant they click it.

## Invariant (pinned)

The frontend has no JS test runner (every frontend unit is verified by the
standing gate), so the pinned check is:

- **Gate:** `npx tsc --noEmit` (exit 0) + `npm run build` (exit 0, no new errors).
- **Behavioral pin:** `src/main.ts` — `applyRenderLOD(n)` flips node resolution,
  semantic particles, and (until `userToggledBundle`) merged bundling on the
  `n >= LARGE_GRAPH_NODES` boundary, and is called at every site that also calls
  `applyForceTuning`. `SEMANTIC_PARTICLES` is the named accessor it suppresses and
  restores. `enableNodeDrag(false)` is set in the `ForceGraph3D` builder chain. If
  large-graph perf or the drag warning regresses, these are where it shows.
- **Deferred lever (open Catch-all):** per-node `InstancedMesh` — do it with
  interactive testing in the loop; it must preserve click selection (raycast
  `instanceId`), hover, per-instance colour/scale, and the click-vs-orbit pointer
  disambiguation.

## Anchors

- `src/main.ts` — `SEMANTIC_PARTICLES`, `LARGE_GRAPH_NODES`, `userToggledBundle`,
  `applyRenderLOD`, its call sites (alongside `applyForceTuning`), the
  `userToggledBundle` set in the bundle-toggle handler, and `.enableNodeDrag(false)`
  in the graph builder.
- `src/bundling.ts` — the `EdgeBundle` (ADR-0009) that auto-bundling engages.
