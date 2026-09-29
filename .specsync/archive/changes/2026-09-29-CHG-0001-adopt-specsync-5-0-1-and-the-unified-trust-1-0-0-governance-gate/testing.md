---
change: CHG-0001-adopt-specsync-5-0-1-and-the-unified-trust-1-0-0-governance-gate
artifact: testing
---

# Testing

- `specsync check --strict --require-coverage 100 --force`
- `specsync agents status`
- `fledge trust doctor`
- `fledge lanes run verify`
- Existing debug/release build and test workflows, lint workflow, and docs build

## Requirement evidence

- `REQ-nano-cli-001`: the workspace test lane exercises Nano startup/configuration, wallet and keystore
  validation, message storage, runtime dispatch, CLI error paths, and orderly runtime shutdown.
- `REQ-plugin-cli-001`: `corvid-plugin-cli` unit tests exercise scaffold creation and rejection paths plus
  manifest, capability-tier, and WASM validation.
- `REQ-plugin-host-001`: `corvid-plugin-host` unit and E2E tests exercise capability enforcement, sandbox and
  SSRF boundaries, signed-plugin loading, invocation failures, registry lifecycle, and hot reload.
- `REQ-plugin-macros-001`: the macro unit tests and workspace compilation verify attribute parsing and generated
  SDK/WASM integration; compile-time expansion failures fail the lane.
- `REQ-plugin-personality-001`: the personality plugin tests exercise provider validation, persisted configuration,
  tool schemas, chat configuration errors, persona state, and manifest round trips.
- `REQ-plugin-sdk-001`: SDK unit tests exercise ABI consistency, manifest serialization, capabilities, events,
  errors, and tool payload round trips; host E2E tests exercise those contracts across the WASM boundary.
- `REQ-transaction-001`: Nano transaction tests exercise payment construction, address validation and round trips,
  deterministic signing, and signed-envelope encoding.
