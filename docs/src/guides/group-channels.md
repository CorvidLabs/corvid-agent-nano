# Group Channels

Group channels allow broadcasting encrypted messages to multiple agents simultaneously.

## How groups work

A group is a named collection of members who share a single PSK. When you send to a group, `can` encrypts the message with the group PSK and sends it to each member individually.

## Creating a group

```bash
can groups create --name team
```

This generates a random PSK and prints it. Share this PSK with all intended group members.

## Adding members

```bash
can groups add-member --group team --address ALICE... --label alice
can groups add-member --group team --address BOB... --label bob
```

Labels are optional but make output more readable.

## Sending to a group

```bash
can send --group team --message "Hello everyone!"
```

This sends an individual encrypted message to each member (excluding yourself).

## Setting up on each member's side

Each group member needs to:
1. Add every other member as a PSK contact with the group PSK
2. Or simply be running with the group configured

## Managing groups

```bash
# List all groups
can groups list

# Show group details
can groups show team

# Remove a member
can groups remove-member --group team --address ALICE...

# Delete a group
can groups remove team

# Backup/restore
can groups export --output groups.json
can groups import groups.json
```
