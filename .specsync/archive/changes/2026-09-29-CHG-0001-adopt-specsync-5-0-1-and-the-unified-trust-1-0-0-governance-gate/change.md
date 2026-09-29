---
id: CHG-0001-adopt-specsync-5-0-1-and-the-unified-trust-1-0-0-governance-gate
state: archived
type: migration
base_commit: 9eb6a906bdb501c141a058b3d014d02fd32cfbad
---

# Adopt SpecSync 5.0.1 and the unified Trust 1.0.0 governance gate

## Intent

Adopt SpecSync 5.0.1 and the unified Trust 1.0.0 governance gate

## Affected Canonical Specs

- `nano-cli`
- `transaction`
- `plugin-cli`
- `plugin-host`
- `plugin-macros`
- `plugin-personality`
- `plugin-sdk`

## Acceptance Criteria

- SpecSync strict check passes; all four agent integrations are installed; the immutable Trust 1.0.0 action runs as job trust; Rust formatting, clippy, workspace tests, and workspace builds remain green; standalone documentation Pages and Atlas publishing remain independent

## No-spec Rationale

Not applicable

## Migration Note

Migrated by hand to SpecSync 6 per Leif's decision (2026-09-28); the 6.0.0 tool refused to archive this legacy record (`` exact-only delivery input `.github/workflows/trust.yml` changed after acceptance and requires an audited reopen; run `specsync change reopen CHG-0001-adopt-specsync-5-0-1-and-the-unified-trust-1-0-0-governance-gate` to re-verify the accepted change, or supersede it from a later change under a module granted the path by `owns` in `.specsync/config.toml` ``).

- Workflow v1 (SpecSync 5) record, accepted on 2026-07-14 by the closing approval already stored in `approvals.json`. SpecSync 6.0.0 reports its accepted evidence as stale, for the reason quoted above.
- Moved by hand from `.specsync/changes/CHG-0001-adopt-specsync-5-0-1-and-the-unified-trust-1-0-0-governance-gate/` into the layout `specsync change archive` writes: `accepted-state.json` is the unchanged accepted `state.json`, `state.json` is marked `archived`, and this file's front matter says `archived`.
- `approvals.json`, `verification.json`, `verification-attempts.json` and every other artifact are the original SpecSync 5 evidence, unchanged. `verification.json` verifies commit `38c0adff05d33123fdc761cbbf3bdaadb0b9576b`, not the tree this record was archived from.
- This migration added no verification evidence, test result, attempt history, or approval. It is a manual migration, not a fresh re-verification.
- Closing it through the tool takes `specsync change reopen`, `specsync change verify`, then `specsync change accept`, which writes a new closing approval. Per Leif's decision it was archived by hand instead, so no reopen or new approval is recorded.
