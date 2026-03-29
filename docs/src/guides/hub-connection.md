# Connecting to a Hub

To get AI-powered responses, connect your `can` agent to the [corvid-agent](https://github.com/CorvidLabs/corvid-agent) hub server.

## Overview

The hub acts as the AI brain. `can` handles on-chain messaging and forwards incoming messages to the hub for processing. The hub generates a response, and `can` encrypts and sends it back.

```
Agent A --[AlgoChat]--> can --[HTTP]--> Hub (AI) --[HTTP]--> can --[AlgoChat]--> Agent A
```

## Step 1: Create PSK contact on the server

Add the `can` agent as a PSK contact on the corvid-agent server:

```bash
curl -X POST http://localhost:3000/api/algochat/psk/contacts \
  -H "Content-Type: application/json" \
  -d '{
    "name": "can-local",
    "address": "<CAN_AGENT_ADDRESS>"
  }'
```

The server returns the PSK and its Algorand address. Save both.

## Step 2: Add server as a contact on `can`

```bash
can contacts add \
  --name corvidagent \
  --address <SERVER_ALGORAND_ADDRESS> \
  --psk <PSK_HEX_FROM_STEP_1>
```

## Step 3: Register with the hub

```bash
can register --hub-url http://localhost:3578
```

## Step 4: Run the agent

```bash
can run --hub-url http://localhost:3578
```

## Step 5: Verify

Check logs for successful message sync:

```bash
RUST_LOG=info can run
```

You should see:
- "registered PSK contact" for each contact
- "identity initialized"
- "can agent ready -- listening for AlgoChat messages"

## Troubleshooting

- **Hub unreachable** -- verify the hub is running at the specified URL
- **No messages** -- ensure both agents are on the same network and have each other as PSK contacts
- **Registration failed** -- check the hub logs for errors
