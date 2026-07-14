## ADDED

### REQUIREMENT REQ-plugin-macros-001

Plugin macros SHALL generate the documented WASM exports and serialization bridge for annotated plugin types.

Acceptance Criteria

- Generated initialization and tool-routing exports encode inputs and outputs using the SDK payload format.
- Unknown tool names and invalid payloads return encoded errors without invoking plugin logic.
