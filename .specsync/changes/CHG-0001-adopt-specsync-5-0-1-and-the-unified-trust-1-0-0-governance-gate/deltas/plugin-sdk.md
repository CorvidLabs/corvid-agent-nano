## ADDED

### REQUIREMENT REQ-plugin-sdk-001

The plugin SDK SHALL expose the documented manifest, capability, context, tool, service, event, error, and host-API
contracts used by guest plugins and the host runtime.

Acceptance Criteria

- Manifest validation rejects unknown or internally inconsistent capability declarations.
- Host API calls and service handles preserve typed success and failure results across the WASM boundary.
