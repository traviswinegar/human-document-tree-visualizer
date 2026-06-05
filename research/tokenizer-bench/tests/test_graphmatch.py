"""Test-first checks for the (b) graph-match scorer (Phase 11 / ADR-00019 pinned
invariant). The metric is the fairness core, so it is unit-tested on hand-computable
fixtures with no model. Runs under pytest, or directly:

    python tests/test_graphmatch.py
"""

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from tokbench.graphmatch import (  # noqa: E402
    graph_json,
    parse_term_graph,
    prf1,
    score_predictions,
)


def test_prf1_basic():
    p, r, f1 = prf1({"a", "b"}, {"b", "c"})
    assert p == 0.5 and r == 0.5 and abs(f1 - 0.5) < 1e-12


def test_prf1_edges_empty_and_vacuous():
    assert prf1(set(), {"a"}) == (0.0, 0.0, 0.0)       # emitted nothing → 0
    assert prf1({"a"}, set()) == (0.0, 0.0, 0.0)       # nothing to match → 0
    assert prf1(set(), set()) == (1.0, 1.0, 1.0)       # vacuous correct-nothing


def test_prf1_perfect():
    assert prf1({"a", "b"}, {"a", "b"}) == (1.0, 1.0, 1.0)


def test_parse_valid_graph():
    parsed = parse_term_graph('{"nodes": ["mara", "vane"], "edges": [["mara", "vane"]]}')
    assert parsed is not None
    nodes, edges = parsed
    assert nodes == {"mara", "vane"}
    assert edges == {frozenset(("mara", "vane"))}


def test_parse_strips_model_preamble_and_trailing():
    # decoder-only models emit junk around the object; we slice first { .. last }.
    txt = 'sure! {"nodes": ["a"], "edges": []} <eos> blah'
    parsed = parse_term_graph(txt)
    assert parsed == ({"a"}, set())


def test_parse_rejects_invalid():
    assert parse_term_graph("") is None
    assert parse_term_graph("no braces here") is None
    assert parse_term_graph('{"nodes": [1,2], "edges": []}') is None      # non-string node
    assert parse_term_graph('{"nodes": ["a"]}') is None                    # missing edges
    assert parse_term_graph('{"nodes": ["a"], "edges": [["a"]]}') is None  # bad pair arity
    assert parse_term_graph('{"nodes": ["a"], "edges": "x"}') is None      # edges not a list
    assert parse_term_graph('{"nodes": ["a", broken') is None              # invalid JSON


def test_parse_drops_self_loops():
    parsed = parse_term_graph('{"nodes": ["a","b"], "edges": [["a","a"],["a","b"]]}')
    assert parsed == ({"a", "b"}, {frozenset(("a", "b"))})


def test_graph_json_is_canonical_and_round_trips():
    s = graph_json(["vane", "mara", "mara"], [("vane", "mara"), ("mara", "vane")])
    # sorted nodes, deduped + sorted undirected edges, compact
    assert s == '{"nodes":["mara","vane"],"edges":[["mara","vane"]]}'
    assert parse_term_graph(s) == ({"mara", "vane"}, {frozenset(("mara", "vane"))})


def test_score_predictions_aggregate():
    truths = [
        ({"a", "b"}, {frozenset(("a", "b"))}),
        ({"x", "y"}, set()),
    ]
    preds = [
        '{"nodes":["a","b"],"edges":[["a","b"]]}',   # perfect → node_f1 1, edge_f1 1
        "garbage, no json",                            # invalid → 0, not valid
    ]
    s = score_predictions(preds, truths)
    assert s["n"] == 2
    assert s["valid_rate"] == 0.5
    assert abs(s["node_f1"] - 0.5) < 1e-12   # (1.0 + 0) / 2
    assert abs(s["edge_f1"] - 0.5) < 1e-12   # (1.0 + 0) / 2


def _run_all():
    fns = [v for k, v in sorted(globals().items()) if k.startswith("test_") and callable(v)]
    for fn in fns:
        fn()
        print(f"ok  {fn.__name__}")
    print(f"\n{len(fns)} passed")


if __name__ == "__main__":
    _run_all()
