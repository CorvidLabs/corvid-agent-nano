# change-password

Change the keystore encryption password.

```bash
can change-password [OPTIONS]
```

## Options

| Flag | Description |
|------|-------------|
| `--old-password` | Current password (prompts if not provided) |
| `--new-password` | New password (prompts if not provided) |

## Examples

```bash
# Interactive (prompts for both passwords)
can change-password

# Non-interactive
can change-password --old-password "oldpass123" --new-password "newpass456"
```

## Notes

- New password must be at least 8 characters
- The keystore is re-encrypted in place (atomic write)
- The underlying seed and address do not change
