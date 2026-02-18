# Channel Adapter Guide

A **channel adapter** (also called a "connector") is any service that bridges a
messaging platform (Telegram, Discord, WhatsApp, Slack, IRC, etc.) to
SerialAgent's gateway.  Adapters normalize incoming messages into a single JSON
envelope and POST them to `POST /v1/inbound`.  The gateway processes the message,
runs a turn against the LLM, and returns outbound actions that the adapter
translates back into platform-native API calls.

```
Platform (Telegram, Discord, ...)
        |
        v
  Channel Adapter          POST /v1/inbound
  (your service)  ────────────────────────> SerialAgent Gateway
                  <────────────────────────
                     JSON response with
                     outbound actions
        |
        v
Platform (send reply, typing indicator, reaction, ...)
```

---

## InboundEnvelope format

The request body is a JSON object called `InboundEnvelope`.  All fields marked
**(required)** must be present; everything else is optional and additive.

### Core fields

| Field         | Type     | Required | Description |
|---------------|----------|----------|-------------|
| `channel`     | string   | yes      | Connector name: `"telegram"`, `"discord"`, `"whatsapp"`, `"slack"`, etc. |
| `peer_id`     | string   | yes      | Raw peer ID of the sender. Should be provider-prefixed: `"telegram:123456"`, `"discord:98765"`. |
| `text`        | string   | yes      | The user's message text. |
| `chat_type`   | string   | no       | `"direct"` (default), `"group"`, `"channel"`, `"thread"`, or `"topic"`. |
| `account_id`  | string   | no       | Bot account ID within the connector (for multi-bot setups). |
| `chat_id`     | string   | **yes for non-DM** | Reply container ID. Discord channel ID, Telegram chat ID, WhatsApp JID. Required when `chat_type` is not `"direct"`. |
| `group_id`    | string   | no       | Space / server / workspace / guild ID. Only needed when channel IDs are not globally unique (e.g. Slack, Teams). |
| `thread_id`   | string   | no       | Thread or topic ID within the chat container. |
| `model`       | string   | no       | Override the model used for this turn. |
| `display`     | object   | no       | Display metadata (logging/dashboard only). See below. |
| `attachments` | array    | no       | Reserved for future use. |

### V1 fields (additive, all optional)

| Field                  | Type     | Description |
|------------------------|----------|-------------|
| `v`                    | integer  | Envelope version. `1` for v1 with new fields. Omit for legacy. |
| `event_id`             | string   | Idempotency key. Recommended format: `"{channel}:{account_id}:{message_id}"`. Prevents duplicate processing from webhook retries. |
| `event_type`           | string   | Event type. Currently only `"message.create"` is processed; all others are acknowledged but skipped. |
| `ts`                   | string   | Event timestamp (ISO 8601). |
| `message_id`           | string   | Platform-native message ID. |
| `reply_to_message_id`  | string   | Message being replied to (for threading/reply context). |
| `mentions`             | array    | Mentioned users / roles / channels. Each entry: `{ "kind": "user"|"role"|"channel", "id": "...", "display": "..." }`. |
| `delivery`             | object   | Delivery capabilities and constraints. See below. |
| `trace`                | object   | Tracing / correlation metadata. See below. |

### Nested objects

**`display`**:
```json
{
  "sender_name": "Alice",
  "room_name": "#general"
}
```

**`delivery`**:
```json
{
  "expects_reply": true,
  "max_reply_chars": 2000,
  "supports_markdown": true,
  "supports_typing": true
}
```

- `max_reply_chars` -- The gateway splits long replies at paragraph, sentence, or word boundaries to fit within this limit. Discord adapters should set this to `2000`.
- `supports_typing` -- When `true`, the response includes a `send.typing` action before the message actions.
- `supports_markdown` -- When `true` (default), replies use `"markdown"` format; otherwise `"plain"`.

**`trace`**:
```json
{
  "request_id": "req-abc-123",
  "connector_id": "telegram-worker-1"
}
```

### Minimal DM example

```json
{
  "channel": "telegram",
  "peer_id": "telegram:123456",
  "text": "Hello, what is the weather today?",
  "chat_type": "direct"
}
```

### Full group message example

```json
{
  "v": 1,
  "channel": "discord",
  "account_id": "bot-main",
  "peer_id": "discord:98765",
  "chat_type": "group",
  "chat_id": "1234567890",
  "group_id": "guild-42",
  "thread_id": null,
  "text": "@SerialAgent summarize the last hour",
  "event_id": "discord:bot-main:msg-abc-123",
  "event_type": "message.create",
  "ts": "2026-02-18T14:30:00Z",
  "message_id": "msg-abc-123",
  "display": {
    "sender_name": "Alice",
    "room_name": "#general"
  },
  "delivery": {
    "expects_reply": true,
    "max_reply_chars": 2000,
    "supports_markdown": true,
    "supports_typing": true
  }
}
```

---

## Session resolution

SerialAgent uses a deterministic session key model (aligned with OpenClaw) to
route messages to sessions.  The session key is computed from the `InboundEnvelope`
fields, **not** managed by the adapter.

### Session key templates

**DM sessions** (controlled by the `dm_scope` config):

| DM scope                    | Key template                                                 |
|-----------------------------|--------------------------------------------------------------|
| `main`                      | `agent:<agentId>:main`                                       |
| `per_peer`                  | `agent:<agentId>:dm:<peerId>`                                |
| `per_channel_peer` (default)| `agent:<agentId>:<channel>:dm:<peerId>`                      |
| `per_account_channel_peer`  | `agent:<agentId>:<channel>:<accountId>:dm:<peerId>`          |

**Group sessions** (independent of `dm_scope`):

| Scenario                  | Key template                                                   |
|---------------------------|----------------------------------------------------------------|
| Unscoped group            | `agent:<agentId>:<channel>:group:<chatId>`                     |
| Scoped group (guild)      | `agent:<agentId>:<channel>:group:<groupId>:<chatId>`           |
| With thread               | `...group:...:<chatId>:thread:<threadId>`                      |

### Key rules for connector authors

1. `channel` and `account_id` are normalized to lowercase.
2. `peer_id` is resolved through the identity resolver before key computation.
3. Non-DM messages **must** include `chat_id` (the reply container). The gateway returns `400 Bad Request` if missing.
4. `group_id` is optional scoping for platforms where channel IDs are not globally unique (Slack, Teams). Discord channel IDs are globally unique, so `group_id` is optional but recommended.
5. `thread_id` is appended only to non-DM keys. It is ignored for DMs.
6. DM sessions never include `group_id` or `thread_id` in the key.

### DM scope explained

The default scope `per_channel_peer` means that Alice talking to the bot on
Telegram gets a different session than Alice talking on Discord.  This is the
safe default for multi-user inboxes because it prevents cross-user context leakage.

To merge Alice's sessions across channels, use identity linking (see below) with
`dm_scope: per_peer` or `dm_scope: main`.

---

## Identity linking

Identity linking allows the same human using different platforms to share a
single canonical identity (and therefore the same DM session, depending on
`dm_scope`).

### Configuration

In `serial-agent.toml`:

```toml
[sessions]
agent_id = "my-bot"
dm_scope = "per_peer"   # or "per_channel_peer" (default)

[[sessions.identity_links]]
canonical = "alice"
peer_ids = [
  "telegram:123456",
  "discord:98765",
  "whatsapp:+33612345678",
]

[[sessions.identity_links]]
canonical = "bob"
peer_ids = [
  "telegram:789012",
  "discord:54321",
]
```

### How it works

1. The adapter sends `peer_id = "telegram:123456"` in the envelope.
2. The gateway's `IdentityResolver` looks up the peer ID in the configured links.
3. If a match is found, the canonical identity (`"alice"`) replaces the raw peer ID in session key computation.
4. With `dm_scope: per_peer`, Alice gets session key `agent:my-bot:dm:alice` regardless of which platform she uses.
5. With `dm_scope: per_channel_peer` (default), Alice still gets separate sessions per channel: `agent:my-bot:telegram:dm:alice` and `agent:my-bot:discord:dm:alice`.

If no identity link matches, the raw `peer_id` is used as-is.

---

## Authentication

The `/v1/inbound` endpoint is **protected** by bearer-token authentication.

### Setup

1. Set the `SA_API_TOKEN` environment variable on the gateway:

   ```bash
   export SA_API_TOKEN="your-secret-token-here"
   ```

2. Include the token in every adapter request:

   ```
   Authorization: Bearer your-secret-token-here
   ```

3. If `SA_API_TOKEN` is unset or empty, the gateway runs in **dev mode** and
   allows unauthenticated access (a warning is logged at startup).

The environment variable name can be changed via `server.api_token_env` in the
config file.

### Error response (401)

```json
{
  "error": "invalid or missing API token"
}
```

---

## Response format

A successful response returns HTTP 200 with a JSON body:

```json
{
  "accepted": true,
  "session_key": "agent:my-bot:telegram:dm:telegram:123456",
  "session_id": "550e8400-e29b-41d4-a716-446655440000",
  "actions": [
    {
      "type": "send.typing",
      "chat_id": "123456",
      "ttl_ms": 8000
    },
    {
      "type": "send.message",
      "chat_id": "123456",
      "reply_to_message_id": "msg-original-123",
      "text": "The weather today is sunny with a high of 22C.",
      "format": "markdown"
    }
  ]
}
```

### Response fields

| Field          | Type    | Description |
|----------------|---------|-------------|
| `accepted`     | bool    | Always `true` for successfully processed requests. |
| `deduped`      | bool    | `true` if the `event_id` was already seen (duplicate delivery). Only present when `true`. |
| `session_key`  | string  | The computed session key for this message. |
| `session_id`   | string  | UUID of the session instance. |
| `actions`      | array   | Outbound actions the adapter should execute (see below). |
| `policy`       | string  | Present when a policy blocked processing: `"denied:channel"`, `"denied:group"`, `"deduped"`, `"stopped"`, or `"unsupported_event:<type>"`. |
| `telemetry`    | object  | Token usage: `{ "input_tokens": 150, "output_tokens": 42 }`. Omitted when zero. |

### Outbound action types

Each action has a `type` field and relevant payload fields.  `null`/absent fields are omitted from the JSON.

**`send.message`** -- Send a text message:
```json
{
  "type": "send.message",
  "chat_id": "123456",
  "thread_id": null,
  "reply_to_message_id": "msg-original-123",
  "text": "Hello!",
  "format": "markdown"
}
```

**`send.typing`** -- Show a typing indicator:
```json
{
  "type": "send.typing",
  "chat_id": "123456",
  "ttl_ms": 8000
}
```

**`react.add`** -- Add a reaction to a message:
```json
{
  "type": "react.add",
  "chat_id": "123456",
  "message_id": "msg-789",
  "emoji": "\u2705"
}
```

### Reply splitting

When `delivery.max_reply_chars` is set and the LLM response exceeds that limit,
the gateway automatically splits the reply into multiple `send.message` actions.
Splitting prefers natural boundaries (paragraph breaks, sentences, word
boundaries).  Only the first chunk includes `reply_to_message_id`.

### Error responses

| HTTP Status | Condition |
|-------------|-----------|
| `400`       | Missing `chat_id` for non-direct message. |
| `401`       | Invalid or missing API token. |
| `429`       | Session is busy (a turn is already in progress for this session key). |
| `500`       | LLM or internal error during turn execution. |

Error body format:
```json
{
  "error": "description of the error",
  "session_key": "agent:my-bot:telegram:dm:telegram:123456"
}
```

---

## Example: Telegram adapter (Python)

A minimal Telegram adapter using `python-telegram-bot` and `httpx`:

```python
"""
Telegram channel adapter for SerialAgent.

Requirements:
    pip install python-telegram-bot httpx

Environment variables:
    TELEGRAM_BOT_TOKEN  -- Telegram bot token from @BotFather
    SA_GATEWAY_URL      -- SerialAgent gateway URL (default: http://localhost:3210)
    SA_API_TOKEN        -- API bearer token for the gateway
"""

import os
import asyncio
import logging

import httpx
from telegram import Update
from telegram.ext import (
    ApplicationBuilder,
    CommandHandler,
    MessageHandler,
    ContextTypes,
    filters,
)

GATEWAY_URL = os.environ.get("SA_GATEWAY_URL", "http://localhost:3210")
API_TOKEN = os.environ.get("SA_API_TOKEN", "")
BOT_TOKEN = os.environ["TELEGRAM_BOT_TOKEN"]

logging.basicConfig(level=logging.INFO)
logger = logging.getLogger(__name__)

http_client = httpx.AsyncClient(timeout=120.0)


def build_envelope(update: Update) -> dict:
    """Build an InboundEnvelope from a Telegram Update."""
    message = update.effective_message
    chat = update.effective_chat
    user = update.effective_user

    is_direct = chat.type == "private"

    envelope = {
        "v": 1,
        "channel": "telegram",
        "peer_id": f"telegram:{user.id}",
        "text": message.text or "",
        "chat_type": "direct" if is_direct else "group",
        "event_id": f"telegram::{message.message_id}",
        "event_type": "message.create",
        "message_id": str(message.message_id),
        "display": {
            "sender_name": user.full_name,
            "room_name": chat.title,
        },
        "delivery": {
            "expects_reply": True,
            "max_reply_chars": 4096,
            "supports_markdown": True,
            "supports_typing": True,
        },
    }

    if not is_direct:
        envelope["chat_id"] = str(chat.id)
        if message.message_thread_id:
            envelope["thread_id"] = str(message.message_thread_id)

    if message.reply_to_message:
        envelope["reply_to_message_id"] = str(
            message.reply_to_message.message_id
        )

    return envelope


async def handle_message(
    update: Update, context: ContextTypes.DEFAULT_TYPE
) -> None:
    """Forward a Telegram message to SerialAgent and relay the response."""
    envelope = build_envelope(update)
    chat_id = update.effective_chat.id

    headers = {"Content-Type": "application/json"}
    if API_TOKEN:
        headers["Authorization"] = f"Bearer {API_TOKEN}"

    try:
        resp = await http_client.post(
            f"{GATEWAY_URL}/v1/inbound",
            json=envelope,
            headers=headers,
        )
        resp.raise_for_status()
        data = resp.json()
    except httpx.HTTPStatusError as exc:
        logger.error("Gateway returned %s: %s", exc.response.status_code, exc.response.text)
        await context.bot.send_message(chat_id, "Sorry, something went wrong.")
        return
    except httpx.RequestError as exc:
        logger.error("Gateway request failed: %s", exc)
        await context.bot.send_message(chat_id, "Sorry, I could not reach the server.")
        return

    # Process outbound actions.
    for action in data.get("actions", []):
        action_type = action.get("type")

        if action_type == "send.typing":
            await context.bot.send_chat_action(chat_id, "typing")

        elif action_type == "send.message":
            reply_to = action.get("reply_to_message_id")
            parse_mode = "Markdown" if action.get("format") == "markdown" else None
            await context.bot.send_message(
                chat_id,
                action["text"],
                reply_to_message_id=int(reply_to) if reply_to else None,
                parse_mode=parse_mode,
            )

        elif action_type == "react.add":
            # Telegram reactions require the message_id.
            msg_id = action.get("message_id")
            if msg_id:
                logger.info("Would react with %s to message %s", action.get("emoji"), msg_id)


async def start_command(
    update: Update, context: ContextTypes.DEFAULT_TYPE
) -> None:
    """Handle the /start command."""
    await update.message.reply_text(
        "Hello! I am connected to SerialAgent. Send me a message."
    )


def main() -> None:
    app = ApplicationBuilder().token(BOT_TOKEN).build()
    app.add_handler(CommandHandler("start", start_command))
    app.add_handler(MessageHandler(filters.TEXT & ~filters.COMMAND, handle_message))

    logger.info("Telegram adapter starting (gateway: %s)", GATEWAY_URL)
    app.run_polling()


if __name__ == "__main__":
    main()
```

---

## Example: Discord adapter (Python)

A minimal Discord adapter using `discord.py` and `httpx`:

```python
"""
Discord channel adapter for SerialAgent.

Requirements:
    pip install discord.py httpx

Environment variables:
    DISCORD_BOT_TOKEN   -- Discord bot token from the Developer Portal
    SA_GATEWAY_URL      -- SerialAgent gateway URL (default: http://localhost:3210)
    SA_API_TOKEN        -- API bearer token for the gateway
"""

import os
import logging

import discord
import httpx

GATEWAY_URL = os.environ.get("SA_GATEWAY_URL", "http://localhost:3210")
API_TOKEN = os.environ.get("SA_API_TOKEN", "")
BOT_TOKEN = os.environ["DISCORD_BOT_TOKEN"]

logging.basicConfig(level=logging.INFO)
logger = logging.getLogger(__name__)

http_client = httpx.AsyncClient(timeout=120.0)

intents = discord.Intents.default()
intents.message_content = True
client = discord.Client(intents=intents)


def build_envelope(message: discord.Message) -> dict:
    """Build an InboundEnvelope from a Discord message."""
    is_direct = isinstance(message.channel, discord.DMChannel)

    envelope = {
        "v": 1,
        "channel": "discord",
        "account_id": str(client.user.id),
        "peer_id": f"discord:{message.author.id}",
        "text": message.content,
        "chat_type": "direct" if is_direct else "group",
        "event_id": f"discord:{client.user.id}:{message.id}",
        "event_type": "message.create",
        "message_id": str(message.id),
        "ts": message.created_at.isoformat(),
        "display": {
            "sender_name": str(message.author),
            "room_name": getattr(message.channel, "name", "DM"),
        },
        "delivery": {
            "expects_reply": True,
            "max_reply_chars": 2000,
            "supports_markdown": True,
            "supports_typing": True,
        },
    }

    if not is_direct:
        envelope["chat_id"] = str(message.channel.id)
        guild = message.guild
        if guild:
            envelope["group_id"] = str(guild.id)

        # Discord threads
        if isinstance(message.channel, discord.Thread):
            envelope["thread_id"] = str(message.channel.id)
            envelope["chat_id"] = str(message.channel.parent_id)

    if message.reference and message.reference.message_id:
        envelope["reply_to_message_id"] = str(
            message.reference.message_id
        )

    # Extract mentions
    mentions = []
    for user in message.mentions:
        mentions.append({
            "kind": "user",
            "id": str(user.id),
            "display": str(user),
        })
    for role in message.role_mentions:
        mentions.append({
            "kind": "role",
            "id": str(role.id),
            "display": role.name,
        })
    if mentions:
        envelope["mentions"] = mentions

    return envelope


@client.event
async def on_ready():
    logger.info("Discord adapter ready as %s (gateway: %s)", client.user, GATEWAY_URL)


@client.event
async def on_message(message: discord.Message):
    # Ignore messages from the bot itself.
    if message.author == client.user:
        return

    # Ignore messages that do not mention the bot (in group channels).
    is_direct = isinstance(message.channel, discord.DMChannel)
    if not is_direct and client.user not in message.mentions:
        return

    envelope = build_envelope(message)

    headers = {"Content-Type": "application/json"}
    if API_TOKEN:
        headers["Authorization"] = f"Bearer {API_TOKEN}"

    try:
        resp = await http_client.post(
            f"{GATEWAY_URL}/v1/inbound",
            json=envelope,
            headers=headers,
        )
        resp.raise_for_status()
        data = resp.json()
    except httpx.HTTPStatusError as exc:
        logger.error("Gateway returned %s: %s", exc.response.status_code, exc.response.text)
        await message.reply("Sorry, something went wrong.")
        return
    except httpx.RequestError as exc:
        logger.error("Gateway request failed: %s", exc)
        await message.reply("Sorry, I could not reach the server.")
        return

    # Process outbound actions.
    target_channel = message.channel

    for action in data.get("actions", []):
        action_type = action.get("type")

        if action_type == "send.typing":
            async with target_channel.typing():
                pass  # typing indicator is shown while inside this block

        elif action_type == "send.message":
            reply_ref = None
            reply_to = action.get("reply_to_message_id")
            if reply_to:
                reply_ref = discord.MessageReference(
                    message_id=int(reply_to),
                    channel_id=target_channel.id,
                )
            await target_channel.send(
                action["text"],
                reference=reply_ref,
            )

        elif action_type == "react.add":
            msg_id = action.get("message_id")
            emoji = action.get("emoji")
            if msg_id and emoji:
                try:
                    target_msg = await target_channel.fetch_message(int(msg_id))
                    await target_msg.add_reaction(emoji)
                except discord.NotFound:
                    logger.warning("Message %s not found for reaction", msg_id)


def main():
    client.run(BOT_TOKEN)


if __name__ == "__main__":
    main()
```

---

## Send policy

The gateway enforces a **send policy** that controls whether the agent responds
in different contexts.  By default:

- DM messages are **allowed**.
- Group messages are **denied** (`send_policy.deny_groups = true`).

To enable group responses, update `serial-agent.toml`:

```toml
[sessions.send_policy]
deny_groups = false

# Or enable for specific channels only:
[sessions.send_policy.channel_overrides]
discord = "allow"
telegram = "deny"
```

When a message is denied by policy, the response has `accepted: true` but
`actions` is empty, and `policy` indicates the reason (`"denied:group"` or
`"denied:channel"`).

---

## Idempotent delivery

If your adapter might deliver the same message twice (webhook retries, polling
replays, reconnects), include the `event_id` field.  The gateway maintains an
in-memory deduplication store with a configurable TTL.

When a duplicate is detected, the response has:
```json
{
  "accepted": true,
  "deduped": true,
  "session_key": "",
  "session_id": "",
  "actions": [],
  "policy": "deduped"
}
```

Recommended `event_id` format: `"{channel}:{account_id}:{message_id}"`.

---

## Session lifecycle

Sessions can be automatically reset based on lifecycle rules in the config:

```toml
[sessions.lifecycle]
daily_reset_hour = 4        # Reset all sessions at 4 AM
idle_minutes = 120           # Reset after 2 hours of inactivity

[sessions.lifecycle.reset_by_channel.telegram]
idle_minutes = 60            # Telegram sessions reset after 1 hour

[sessions.lifecycle.reset_by_type.group]
daily_reset_hour = 0         # Group sessions reset at midnight
```

When a session is reset, a new `session_id` is minted for the same
`session_key`.  The adapter does not need to handle this -- the gateway manages
it transparently.

---

## Checklist for building a new adapter

1. **Listen** for messages on the platform (webhook, polling, WebSocket).
2. **Normalize** each message into an `InboundEnvelope` JSON object.
3. **POST** the envelope to `POST /v1/inbound` with `Authorization: Bearer <token>`.
4. **Process** each action in the `actions` array of the response:
   - `send.typing` -- trigger a typing indicator.
   - `send.message` -- send the message text to the target chat. Handle `reply_to_message_id` for threading. Handle `format` for markdown vs. plain text.
   - `react.add` -- add a reaction to the specified message.
5. **Handle errors** -- log and gracefully degrade on 4xx/5xx responses.
6. **Set `event_id`** for idempotent delivery if the platform may retry.
7. **Set `delivery` hints** so the gateway can split replies and send typing indicators appropriately.
8. **Prefix `peer_id`** with the channel name (e.g. `"telegram:123"`) to avoid collisions across platforms.
