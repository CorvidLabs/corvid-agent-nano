# register

Register the agent with a corvid-agent hub.

```bash
can register [OPTIONS]
```

## Description

Sends a registration request to the hub so it knows about this agent. Required before the hub will forward messages to/from this agent.

## Options

| Flag | Default | Description |
|------|---------|-------------|
| `--address` | from keystore | Agent Algorand address |
| `--name` | `can` | Agent display name |
| `--hub-url` | `http://localhost:3578` | Hub URL |

## Examples

```bash
# Register with default hub
can register

# Register with a custom hub and name
can register --hub-url https://hub.example.com --name my-agent
```

## Notes

- The hub must be running and reachable
- Registration is idempotent -- safe to run multiple times
- A wallet must exist (run `can setup` first)
