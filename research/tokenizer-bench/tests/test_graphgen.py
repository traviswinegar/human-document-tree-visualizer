"""Test-first checks for the (b) generation data pipeline (Phase 11 / ADR-00019).
Pure helpers only (the torch trainer/generator are imported lazily inside graphgen.main),
so this runs under the global Python with no GPU stack. Directly:

    python tests/test_graphgen.py
"""

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from tokbench.graphgen import ByteTok, build_example, make_tok  # noqa: E402


def test_byte_tok_round_trips():
    tok = ByteTok()
    assert tok.base == 256
    s = "Mara — dragon café"
    ids = tok.encode_str(s)
    assert all(0 <= i < 256 for i in ids)
    assert tok.decode(ids) == s


def test_build_example_shapes_text_sep_graph_eos():
    tok = ByteTok()
    sep, eos = tok.base, tok.base + 1
    pair = (b"the dragon and the gold", {"nodes": ["dragon", "gold"], "edges": [["dragon", "gold"]]})
    out = build_example(pair, tok, block=512, min_nodes=2)
    assert out is not None
    ids, sep_pos = out
    assert ids[sep_pos] == sep, "SEP sits at sep_pos"
    assert ids[-1] == eos, "sequence ends with EOS"
    # the prefix before SEP decodes back to the source text
    assert tok.decode(ids[:sep_pos]) == "the dragon and the gold"
    # the target region (after SEP, before EOS) parses to the truth graph
    from tokbench.graphmatch import parse_term_graph
    target_text = tok.decode(ids[sep_pos + 1 : -1])
    assert parse_term_graph(target_text) == ({"dragon", "gold"}, {frozenset(("dragon", "gold"))})


def test_build_example_rejects_too_few_nodes():
    tok = ByteTok()
    pair = (b"short", {"nodes": ["solo"], "edges": []})
    assert build_example(pair, tok, block=512, min_nodes=2) is None


def test_build_example_rejects_overlong():
    tok = ByteTok()
    pair = (b"x" * 1000, {"nodes": ["a", "b"], "edges": [["a", "b"]]})
    assert build_example(pair, tok, block=64, min_nodes=2) is None


def test_make_tok_guards_unknown_arm():
    try:
        make_tok("nope")
        raise AssertionError("expected ValueError")
    except ValueError:
        pass


def _run_all():
    fns = [v for k, v in sorted(globals().items()) if k.startswith("test_") and callable(v)]
    for fn in fns:
        fn()
        print(f"ok  {fn.__name__}")
    print(f"\n{len(fns)} passed")


if __name__ == "__main__":
    _run_all()
