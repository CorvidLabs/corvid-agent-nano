# inbox

Read cached messages from the local inbox.

```bash
can inbox [OPTIONS]
```

## Options

| Flag | Default | Description |
|------|---------|-------------|
| `--from <NAME_OR_ADDRESS>` | all | Filter by sender |
| `--limit <N>` | `20` | Maximum messages to display |

## Examples

```bash
# Show last 20 messages
can inbox

# Filter by contact name
can inbox --from alice

# Show more messages
can inbox --limit 50
```

## Output

Messages are displayed in chronological order with:
- **ROUND** -- Algorand round the message was confirmed in
- **DIR** -- Direction: `>>>` (sent) or `<<<` (received)
- **FROM/TO** -- Contact name or truncated address
- **TIME** -- Timestamp
- **MESSAGE** -- Message content (truncated to 60 chars)

## Notes

- Messages are cached locally in `messages.db` when the agent runs
- The inbox only shows cached messages -- run `can run` first to receive messages
- Contact names are resolved automatically if the sender is in your contacts
