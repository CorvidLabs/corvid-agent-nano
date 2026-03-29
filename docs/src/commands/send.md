# send

Send an encrypted message to a contact, address, or group.

```bash
can send --to <RECIPIENT> --message <TEXT> [OPTIONS]
can send --group <GROUP_NAME> --message <TEXT> [OPTIONS]
```

## Options

| Flag | Description |
|------|-------------|
| `--to <NAME_OR_ADDRESS>` | Recipient: contact name or Algorand address |
| `--group <GROUP_NAME>` | Send to all members of a group channel |
| `--message <TEXT>` | Message text to send |
| `--network` | Network preset (default: `localnet`) |
| `--password` | Keystore password |

## Examples

```bash
# Send to a named contact
can send --to alice --message "Hello!"

# Send to a raw address
can send --to ALGO_ADDRESS... --message "Hello!"

# Broadcast to a group
can send --group team --message "Meeting in 5 minutes"
```

## How it works

1. Resolves the recipient (contact name -> address, or validates raw address)
2. Encrypts the message using the PSK shared with that contact
3. Builds an Algorand transaction with the ciphertext in the note field
4. Signs and submits the transaction
5. Displays the transaction ID

For group sends, the message is encrypted and sent individually to each group member.
