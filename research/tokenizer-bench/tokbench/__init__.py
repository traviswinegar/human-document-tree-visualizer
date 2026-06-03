"""tokenizer-bench (Phase 8 / ADR-00015) — the graph-tokenizer training experiment.

Only the pure metric is re-exported here so `import tokbench` stays dependency-light
(no torch / tokenizers pulled in until you import `tokbench.data`).
"""

from .bpb import BYTE_CEIL, bits_per_byte, text_bytes_from_ids, text_nats  # noqa: F401
