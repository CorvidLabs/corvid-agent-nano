# contacts

Manage PSK (pre-shared key) contacts for encrypted messaging.

```bash
can contacts <SUBCOMMAND>
```

## Subcommands

### list

List all contacts.

```bash
can contacts list
```

### add

Add a new contact.

```bash
can contacts add --name <NAME> --address <ALGO_ADDRESS> --psk <KEY> [--force]
```

| Flag | Description |
|------|-------------|
| `--name` | Contact name (used for addressing in `send` and `inbox`) |
| `--address` | Contact's Algorand address (58 chars) |
| `--psk` | Pre-shared key: 64-char hex or 44-char base64 |
| `--force` | Overwrite if contact already exists |

### remove

Remove a contact by name.

```bash
can contacts remove <NAME>
```

### export

Export all contacts to JSON.

```bash
can contacts export [--output <FILE>]
```

Without `--output`, prints to stdout.

### import

Import contacts from a JSON file.

```bash
can contacts import <FILE>
```

## PSK Format

Pre-shared keys can be provided as:
- **Hex**: 64 characters (32 bytes encoded as hex)
- **Base64**: 44 characters (32 bytes encoded as base64)

## Examples

```bash
# Add a contact with hex PSK
can contacts add --name alice --address ALICE... --psk aabbccdd...

# Add with base64 PSK
can contacts add --name bob --address BOB... --psk dGhpcyBpcyBhIHRlc3Qga2V5...

# List all contacts
can contacts list

# Backup and restore
can contacts export --output backup.json
can contacts import backup.json
```
