# config

Manage the `nano.toml` configuration file.

```bash
can config <SUBCOMMAND>
```

## Subcommands

### show

Display the current configuration:

```bash
can config show
```

Prints the full `nano.toml` contents with all sections.

### path

Show the config file path:

```bash
can config path
```

Output: `<data-dir>/nano.toml`

### set

Set a configuration value using dot-separated keys:

```bash
can config set <KEY> <VALUE>
```

**Examples:**

```bash
can config set agent.name "my-agent"
can config set hub.url "http://localhost:3578"
can config set hub.disabled true
can config set runtime.poll_interval 10
can config set runtime.health_port 9090
can config set runtime.no_plugins true
can config set logging.format "json"
can config set logging.level "debug"
```

## Configuration File

The `nano.toml` file lives at `<data-dir>/nano.toml` and supports these sections:

```toml
[agent]
name = "can"                           # Agent display name

[network]
algod_url = "http://localhost:4001"     # Algod endpoint
algod_token = "aaaa..."                # Algod auth token
indexer_url = "http://localhost:8980"   # Indexer endpoint
indexer_token = "aaaa..."              # Indexer auth token

[hub]
url = "http://localhost:3578"          # Hub URL
disabled = false                       # P2P mode (no hub)

[runtime]
poll_interval = 5                      # Seconds between polls
no_plugins = false                     # Disable plugin host
health_port = 9090                     # Health check port (optional)

[logging]
format = "text"                        # "text" or "json"
level = "info"                         # debug, info, warn, error
```

## Precedence

Configuration is resolved in this order (first wins):

1. CLI flags (`--network`, `--hub-url`, etc.)
2. Environment variables (`CAN_NETWORK`, `CAN_PASSWORD`, etc.)
3. `nano.toml` values
4. Built-in defaults

## Examples

```bash
# Set up for testnet with custom hub
can config set agent.name "testnet-agent"
can config set hub.url "https://hub.example.com"
can config set logging.level "debug"

# Enable health monitoring
can config set runtime.health_port 9090

# Switch to P2P mode
can config set hub.disabled true

# Verify
can config show
```
