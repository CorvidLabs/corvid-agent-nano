---
change: CHG-0002-complete-governance-integration-coverage-for-the-specsync-5-and-trust-1-migratio
artifact: testing
---

# Testing

- `specsync check --strict --require-coverage 100 --force`
- `specsync agents status`
- `fledge trust doctor`
- `fledge trust verify`
- Exact-head hosted Trust and existing CodeQL checks after push
