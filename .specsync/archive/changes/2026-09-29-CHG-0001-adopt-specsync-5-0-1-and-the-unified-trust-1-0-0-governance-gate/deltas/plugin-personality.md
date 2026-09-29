## ADDED

### REQUIREMENT REQ-plugin-personality-001

The personality plugin SHALL persist its configuration and conversation state and SHALL expose the documented
configure, chat, emotion, and provider-selection behavior.

Acceptance Criteria

- An unconfigured chat uses documented defaults while configured values survive later invocations.
- Provider or model failures surface as tool errors without corrupting persisted personality state.
