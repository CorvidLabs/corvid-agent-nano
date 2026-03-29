# P2P Mode

Run `can` without a hub for direct agent-to-agent communication.

## What is P2P mode?

By default, `can run` forwards received messages to a corvid-agent hub for AI processing. In P2P mode (`--no-hub`), the agent only receives and stores messages locally -- no hub forwarding.

This is useful for:
- **Message logging** -- archive on-chain messages
- **Edge agents** -- receive commands without AI processing
- **Bridge bots** -- relay messages between platforms
- **Development** -- test messaging without running a hub

## Usage

```bash
can run --no-hub
```

## What works in P2P mode

- Receiving and decrypting AlgoChat messages
- Storing messages in the local cache (`messages.db`)
- Sending messages with `can send`
- Reading the inbox with `can inbox`
- Plugins (if enabled)

## What doesn't work

- Hub-forwarded AI responses (no hub = no AI brain)
- Hub registration (`can register` still works but has no effect in P2P mode)
