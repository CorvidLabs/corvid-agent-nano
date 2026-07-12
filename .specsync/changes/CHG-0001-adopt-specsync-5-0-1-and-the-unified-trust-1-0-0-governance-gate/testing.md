---
change: CHG-0001-adopt-specsync-5-0-1-and-the-unified-trust-1-0-0-governance-gate
artifact: testing
---

# Testing

- `specsync check --strict --require-coverage 0 --force`
- `specsync agents status`
- `fledge trust doctor`
- `fledge lanes run verify`
- Existing debug/release build and test workflows, lint workflow, and docs build
