# Runtime Invariants

## Always
- Emit trace events for every SerialMemory API call.
- Include `session_id` in all memory operations when a session is active.
- Normalize line endings to `\n` before processing workspace files.
- Respect truncation caps — never inject more than `bootstrap_total_max_chars`.

## Never
- Never skip the skills index injection (even if empty, inject the empty block).
- Never cache user facts across sessions — always fetch fresh from SerialMemory.
- Never modify workspace context files at runtime — they are read-only config.

## Health Checks
- On startup, verify SerialMemory connectivity via `/admin/status`.
- If SerialMemory is unreachable, start in degraded mode (no USER_FACTS, log warning).
- Log context build time if it exceeds 500ms.
