"""Graph-match scoring for the (b) downstream task (Phase 11 / ADR-00019).

The (b) milestone replaces bits-per-byte with **the task the product actually does**:
from document text, *generate* the graph. A generated graph is scored against the
deterministic walker graph (the ground truth) on two axes ADR-00019 pins:

* **valid-output rate** — did the model emit something that parses into the expected
  `{nodes, edges}` shape at all? (Tiny from-scratch models often cannot — that is itself
  a designed-for finding, not a bug.)
* **node / edge F1** — precision/recall of the predicted node and edge *sets* against
  the ground-truth sets.

These functions are pure (no torch) so the metric — the experiment's fairness core — is
unit-tested independently of any model, exactly like `bpb.py` (ADR-00019 pinned
invariant). The graph shape used by the downstream task is the **salient-term graph**:
`{"nodes": ["term", ...], "edges": [["a","b"], ...]}` — term nodes (strings) and
*unordered* co-occurrence pairs, both bounded and deterministically derivable from the
walker, so a tiny model has a tractable target.
"""

from __future__ import annotations

import json
from typing import Iterable, Optional, Sequence, Tuple

# A parsed graph: a set of node keys (strings) and a set of edge keys (frozensets of two
# node strings — undirected co-occurrence). Edge endpoints need not appear in `nodes`
# (the scorer compares sets independently), matching how F1 treats the two axes.
ParsedGraph = Tuple[set, set]


def prf1(pred: set, truth: set) -> Tuple[float, float, float]:
    """(precision, recall, F1) of a predicted set against a truth set.

    Conventions at the empty edges (so a model that emits nothing scores 0, and a
    document with no truth items neither rewards nor punishes a non-empty prediction):
    precision is 0 when `pred` is empty; recall is 0 when `truth` is empty; F1 is 0 when
    precision+recall is 0. When BOTH are empty, all three are 1.0 (a vacuous match —
    "correctly predicted nothing").
    """
    if not pred and not truth:
        return 1.0, 1.0, 1.0
    inter = len(pred & truth)
    precision = inter / len(pred) if pred else 0.0
    recall = inter / len(truth) if truth else 0.0
    f1 = (2 * precision * recall / (precision + recall)) if (precision + recall) > 0 else 0.0
    return precision, recall, f1


def parse_term_graph(text: str) -> Optional[ParsedGraph]:
    """Parse generated text into `(nodes, edges)`, or `None` if it is not a valid graph.

    Robust by design: the model's raw output is sliced from the first `{` to the last `}`
    (decoder-only models trail off after the object), then JSON-parsed. Any failure —
    invalid JSON, wrong top-level shape, non-string node, malformed edge pair — returns
    `None` (counts as an invalid generation), never raises.
    """
    if not text:
        return None
    start = text.find("{")
    end = text.rfind("}")
    if start < 0 or end <= start:
        return None
    try:
        obj = json.loads(text[start : end + 1])
    except (json.JSONDecodeError, ValueError):
        return None
    if not isinstance(obj, dict) or "nodes" not in obj or "edges" not in obj:
        return None
    raw_nodes, raw_edges = obj["nodes"], obj["edges"]
    if not isinstance(raw_nodes, list) or not isinstance(raw_edges, list):
        return None
    nodes = set()
    for n in raw_nodes:
        if not isinstance(n, str):
            return None
        nodes.add(n)
    edges = set()
    for e in raw_edges:
        if not isinstance(e, (list, tuple)) or len(e) != 2:
            return None
        a, b = e
        if not isinstance(a, str) or not isinstance(b, str):
            return None
        if a != b:
            edges.add(frozenset((a, b)))
    return nodes, edges


def graph_json(nodes: Iterable[str], edges: Iterable[Tuple[str, str]]) -> str:
    """Serialize a term graph to the canonical compact target JSON (sorted for
    determinism) — the ground-truth string the model is trained to reproduce."""
    ns = sorted(set(nodes))
    es = sorted({tuple(sorted((a, b))) for a, b in edges if a != b})
    return json.dumps({"nodes": ns, "edges": [list(e) for e in es]}, separators=(",", ":"))


def score_predictions(
    predictions: Sequence[str],
    truths: Sequence[ParsedGraph],
) -> dict:
    """Aggregate graph-match metrics over a dataset.

    `predictions` are raw generated strings; `truths` are parsed `(nodes, edges)` ground
    truths. Returns valid-output rate plus **macro** node/edge F1 (per-doc F1 averaged;
    an unparseable prediction contributes 0 to both F1s and to the valid rate).
    """
    if len(predictions) != len(truths):
        raise ValueError("predictions and truths must align 1:1")
    n = len(predictions)
    if n == 0:
        raise ValueError("no predictions to score")
    valid = 0
    node_f1_sum = edge_f1_sum = 0.0
    for pred_text, (tnodes, tedges) in zip(predictions, truths):
        parsed = parse_term_graph(pred_text)
        if parsed is None:
            continue  # invalid → 0 to valid-rate and both F1s
        valid += 1
        pnodes, pedges = parsed
        node_f1_sum += prf1(pnodes, tnodes)[2]
        edge_f1_sum += prf1(pedges, tedges)[2]
    return {
        "n": n,
        "valid_rate": valid / n,
        "node_f1": node_f1_sum / n,
        "edge_f1": edge_f1_sum / n,
    }
