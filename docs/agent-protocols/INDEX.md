# Agent Protocols (human-document-tree)

This project inherits **AgentDNA**. The kernel, operating discipline, and override
channel are stated in the root [`CLAUDE.md`](../../CLAUDE.md) — read that first.

## Local (authoritative copy)

- [`COMPACTION_RECOVERY.md`](COMPACTION_RECOVERY.md) — surviving a context-window
  compaction. Copied verbatim so recovery never depends on a sibling repo being
  present. This is the operative protocol for this project (coding work, commit
  units).

## Canonical corpus (sibling repo — full reference)

The complete corpus lives in the AI Studio repo and is the source of truth for the
framework itself:

`E:\Development\ai-studio\docs\agent-protocols\`

- `INDEX.md` — corpus root, reading stance, topology
- `AGENT_DNA.md` — the human-facing story (the inversion, the kernel, domain
  adaptation incl. the creative-writing mapping relevant to this project)
- `AGENT_DNA_PROTOCOL.md` — agent-facing operational protocol (kernel, seven
  substrate types, six discipline rules, recovery pointer, override channel)
- `COMPACTION_RECOVERY.md` — the original of the local copy here
- `ADVISORY_RECOVERY.md` — decision-ledger protocol for advisory work
  (decision + rationale + cited anchor; Locked/Provisional/Open)

If the sibling path is unavailable on a given machine, the local
`COMPACTION_RECOVERY.md` plus the root `CLAUDE.md` are sufficient to operate.
