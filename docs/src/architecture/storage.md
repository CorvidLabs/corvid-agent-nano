# Data Storage

All persistent data is stored in the `--data-dir` directory (default: `./data`).

## Files

| File | Format | Purpose |
|------|--------|---------|
| `keystore.enc` | JSON | Encrypted wallet (Argon2id + ChaCha20-Poly1305) |
| `contacts.db` | SQLite | PSK contacts |
| `groups.db` | SQLite | Group channels and members |
| `keys.db` | SQLite | AlgoChat key storage (DH session keys) |
| `messages.db` | SQLite | Message cache (inbox) |
| `plugins/` | Directory | WASM plugin files |

## Contacts database

Schema:
```sql
CREATE TABLE contacts (
    name TEXT PRIMARY KEY,
    address TEXT NOT NULL,
    psk BLOB NOT NULL,
    added_at TEXT NOT NULL
);
```

## Groups database

Schema:
```sql
CREATE TABLE groups (
    name TEXT PRIMARY KEY,
    psk BLOB NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE group_members (
    group_name TEXT NOT NULL,
    address TEXT NOT NULL,
    label TEXT,
    added_at TEXT NOT NULL,
    PRIMARY KEY (group_name, address),
    FOREIGN KEY (group_name) REFERENCES groups(name)
);
```

## Message cache

Messages are cached locally when the agent runs. The cache stores:
- Message ID, sender, recipient
- Decrypted content
- Timestamp and confirmed round
- Direction (sent/received)
- Reply-to information

## Backup

```bash
# Backup contacts and groups
can contacts export --output contacts.json
can groups export --output groups.json

# Copy keystore
cp ./data/keystore.enc ~/backup/

# Restore
can contacts import contacts.json
can groups import groups.json
```

## Reset

To completely reset:

```bash
rm -rf ./data
can setup
```
