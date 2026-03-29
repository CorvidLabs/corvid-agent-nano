# groups

Manage group PSK channels for broadcasting messages to multiple agents.

```bash
can groups <SUBCOMMAND>
```

## Subcommands

### create

Create a new group with a random PSK.

```bash
can groups create --name <NAME>
```

Generates a random 32-byte PSK and prints it. Share this PSK with group members.

### list

List all groups.

```bash
can groups list
```

### show

Show group details and members.

```bash
can groups show <NAME>
```

### add-member

Add a member to a group.

```bash
can groups add-member --group <GROUP> --address <ALGO_ADDRESS> [--label <LABEL>]
```

### remove-member

Remove a member from a group.

```bash
can groups remove-member --group <GROUP> --address <ALGO_ADDRESS>
```

### remove

Delete a group and all its members.

```bash
can groups remove <NAME>
```

### export / import

```bash
can groups export [--output <FILE>]
can groups import <FILE>
```

## Example workflow

```bash
# Create a group
can groups create --name team
# Output: PSK: aabbccdd...

# Add members
can groups add-member --group team --address ALICE... --label alice
can groups add-member --group team --address BOB... --label bob

# Broadcast a message
can send --group team --message "Hello team!"

# View group details
can groups show team
```
