"""Bits-per-byte — the fair cross-tokenizer metric for tokenizer-bench (ADR-00015).

Per-token perplexity is **not** comparable across vocabularies (a token covers a
different amount of text in each arm), so every arm is scored in **bits per source
byte**: the cross-entropy (in nats) the model spends predicting the *text*, divided
by ln(2) and by the number of UTF-8 source bytes.

The M1 subtlety (measured: arm C is ~6.25 tok/byte): arm C's stream interleaves text
bytes with structural graph tokens. To measure how well the model predicts the
*text* (not its graph bookkeeping), arm C is scored over its **byte-token positions
only** (ids < 256, one source byte each). Arms A (BPE) and B (byte) score every
predicted token; the denominator is the source byte length those tokens cover —
derivable from the ids for byte-grounded arms (B, C), supplied from the decoded text
for arm A (sub-words span a variable number of bytes).

These functions are pure (no torch) so the metric is unit-tested independently of any
model — the experiment's fairness machinery is what must be correct (ADR-00015
pinned invariant).
"""

from __future__ import annotations

import math
from typing import Iterable, Sequence

# doctree-core tokenizer byte floor: ids 0..255 are literal source bytes (ADR-00013).
BYTE_CEIL = 256


def bits_per_byte(scored_nats: float, text_bytes: int) -> float:
    """`(cross-entropy nats over the scored positions / ln 2) / source text bytes`.

    Normalizing by *source bytes* (not tokens) is what makes arms with different
    vocabularies directly comparable.
    """
    if text_bytes <= 0:
        raise ValueError("text_bytes must be positive")
    return (scored_nats / math.log(2)) / text_bytes


def text_nats(per_token_nats: Sequence[float], target_ids: Sequence[int], arm: str) -> float:
    """Sum the cross-entropy (nats) over the positions that represent *text*.

    arms "C"/"D": only byte-token targets (id < 256) — the structural/coref markers
    (id >= 256) are conditioning context, never scored; arms "A"/"B": every target.
    """
    if arm in ("C", "D"):
        return float(sum(n for n, t in zip(per_token_nats, target_ids) if t < BYTE_CEIL))
    if arm in ("A", "B"):
        return float(sum(per_token_nats))
    raise ValueError(f"unknown arm {arm!r} (expected 'A', 'B', 'C', or 'D')")


def mean_over_body(per_token_nats: Sequence[float], is_body: Sequence[int]) -> float:
    """Mean cross-entropy over body (text) positions only — the *training-loss* form of
    treating the coref/structural markers (`is_body == 0`) as pure conditioning context
    (ADR-00019 (a): "the conditioning prefix contributes zero to the loss"). The model
    attends to the markers (they are inputs) but is never trained to predict them. This
    is the pure reference the torch masked mean in `train.py` mirrors.
    """
    num = 0.0
    den = 0
    for n, b in zip(per_token_nats, is_body):
        if b:
            num += float(n)
            den += 1
    if den == 0:
        raise ValueError("no body positions to average")
    return num / den


def text_bytes_from_ids(target_ids: Iterable[int], arm: str) -> int:
    """Source byte count implied by the ids — valid only for byte-grounded arms.

    arm "B": one byte per token; arms "C"/"D": one byte per byte-token (id < 256). Arm
    "A" is sub-word and not byte-derivable — pass the true source byte count instead.
    """
    if arm in ("C", "D"):
        return sum(1 for t in target_ids if t < BYTE_CEIL)
    if arm == "B":
        return sum(1 for _ in target_ids)
    raise ValueError("arm A is sub-word: supply the true source byte count, not ids")
