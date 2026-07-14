---
change: CHG-0001-adopt-specsync-5-0-1-and-the-unified-trust-1-0-0-governance-gate
artifact: design
---

# Design

Trust uses the standard profile with blocking risk, progressive provenance, and Trust-managed Atlas disabled. The lifecycle lane directly runs formatting, Clippy, workspace tests, and workspace builds. The unified action is pinned to the immutable Trust 1.0.0 commit and replaces only the standalone legacy SpecSync workflow.
