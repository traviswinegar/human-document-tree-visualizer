"""Test-first metric checks for tokenizer-bench (ADR-00015 pinned invariant).

The bits-per-byte machinery is the experiment's fairness core, so it is unit-tested
on hand-computable fixtures, with no model involved. Runs under pytest, or directly:

    python tests/test_bpb.py
"""

import math
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from tokbench.bpb import bits_per_byte, text_bytes_from_ids, text_nats  # noqa: E402


def test_bits_per_byte_basic_and_guard():
    # (ln2 * 8 nats / ln2) / 4 bytes = 8 / 4 = 2.0 bits/byte.
    assert abs(bits_per_byte(math.log(2) * 8, 4) - 2.0) < 1e-12
    try:
        bits_per_byte(1.0, 0)
        raise AssertionError("expected ValueError on text_bytes=0")
    except ValueError:
        pass


def test_uniform_byte_model_scores_8_bits_per_byte():
    # A model uniform over 256 byte values spends exactly log2(256) = 8 bits/byte.
    n = 16
    losses = [math.log(256)] * n            # per-token cross-entropy, nats
    ids = [i % 256 for i in range(n)]       # arm B: all byte tokens
    nats = text_nats(losses, ids, "B")
    nbytes = text_bytes_from_ids(ids, "B")
    assert nbytes == n
    assert abs(bits_per_byte(nats, nbytes) - 8.0) < 1e-9


def test_arm_c_scores_only_byte_positions():
    # Structural tokens (id >= 256) must NOT count toward the text's bits/byte.
    losses = [1.0, 2.0, 3.0, 4.0]
    ids = [65, 300, 66, 301]                # 65,66 = bytes; 300,301 = structural
    assert text_nats(losses, ids, "C") == 4.0          # 1.0 + 3.0 only
    assert text_bytes_from_ids(ids, "C") == 2
    # the two structural positions (nats 2.0, 4.0) are excluded from the numerator
    assert abs(bits_per_byte(4.0, 2) - (4.0 / math.log(2) / 2)) < 1e-12


def test_arm_a_bytes_must_be_supplied():
    # Arm A is sub-word: byte count is not derivable from ids.
    try:
        text_bytes_from_ids([10, 20, 30], "A")
        raise AssertionError("expected ValueError for arm A")
    except ValueError:
        pass
    # but text_nats still sums all sub-word positions
    assert text_nats([1.0, 2.0, 3.0], [10, 20, 30], "A") == 6.0


def _run_all():
    fns = [v for k, v in sorted(globals().items()) if k.startswith("test_") and callable(v)]
    for fn in fns:
        fn()
        print(f"ok  {fn.__name__}")
    print(f"\n{len(fns)} passed")


if __name__ == "__main__":
    _run_all()
