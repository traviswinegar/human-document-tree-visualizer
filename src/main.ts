import ForceGraph3D from "3d-force-graph";
import type { NodeObject, LinkObject } from "3d-force-graph";
import fixtureJson from "../fixtures/sample-narrative.graph.json";
import type { DocGraph, GraphNode, GraphEdge } from "./types";
import { nodeColor, nodeSize, edgeColor } from "./colors";

// The shared fixture (ADR-0001 render target). Imported at build time so the
// scaffold renders something real before the Tauri walker stream exists (A8).
const fixture = fixtureJson as unknown as DocGraph;

// 3d-force-graph's accessors hand back the library's NodeObject/LinkObject; our
// schema props ride along on the same objects, so we narrow with a cast.
const asNode = (n: NodeObject): GraphNode => n as unknown as GraphNode;
const asLink = (l: LinkObject): GraphEdge => l as unknown as GraphEdge;

const container = document.getElementById("graph")!;
const statsEl = document.getElementById("stats")!;

const graph = new ForceGraph3D(container, { controlType: "orbit" })
  .width(window.innerWidth)
  .height(window.innerHeight)
  .backgroundColor("#05070d")
  .graphData({
    nodes: fixture.nodes as unknown as NodeObject[],
    links: fixture.edges as unknown as LinkObject[],
  })
  .nodeColor((n) => nodeColor(asNode(n).kind))
  .nodeVal((n) => nodeSize(asNode(n).kind))
  .nodeLabel((n) => {
    const g = asNode(n);
    return `<b>${g.label}</b><br/><span style="opacity:.7">${g.kind}</span>`;
  })
  .nodeOpacity(0.95)
  .linkColor((l) => edgeColor(asLink(l).kind))
  .linkWidth((l) => (asLink(l).provenance === "semantic" ? 1.2 : 0.4))
  .linkOpacity(0.5);

graph.zoomToFit(0, 60);
statsEl.textContent = `${fixture.nodes.length} nodes · ${fixture.edges.length} edges (fixture)`;

// Dev-only handle so the running 3D scene is inspectable from the page console /
// preview tooling (WebGL canvases can't be verified via readPixels under the
// default preserveDrawingBuffer:false). Stripped from production builds.
if (import.meta.env.DEV) {
  (window as Window & { __doctreeGraph?: typeof graph }).__doctreeGraph = graph;
}

window.addEventListener("resize", () => {
  graph.width(window.innerWidth).height(window.innerHeight);
});
