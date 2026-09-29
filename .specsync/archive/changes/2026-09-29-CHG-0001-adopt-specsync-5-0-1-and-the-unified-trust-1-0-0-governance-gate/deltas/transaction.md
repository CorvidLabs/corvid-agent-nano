## ADDED

### REQUIREMENT REQ-transaction-001

Transaction helpers SHALL construct, sign, submit, and decode AlgoChat transactions using the documented Algorand
address and signed-envelope formats.

Acceptance Criteria

- Invalid addresses and malformed envelopes return errors rather than producing a transaction.
- Address and payload round trips preserve the original public data.
