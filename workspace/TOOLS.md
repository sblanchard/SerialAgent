# Tool Usage Policy

## SerialMemory Tools
- **memory_search**: Use hybrid mode by default. Lower threshold to 0.5 for exploratory queries.
- **memory_ingest**: Always set `extract_entities: true`. Use dedup_mode `warn` to surface duplicates.
- **memory_about_user**: Call at session start to load user context.
- **instantiate_context**: Use for project-scoped sessions with `days_back: 7` for active projects.
- **multi_hop_search**: Use when the question involves relationships (e.g., "who worked on X with Y?").

## Safety Rules
- Never call ADMIN-risk tools without explicit user confirmation.
- Never ingest PII (passwords, tokens, keys) into memory.
- Rate-limit awareness: if you get a 429, back off and inform the user.
- Always provide `reason` when updating or deleting memories.

## When NOT to Use Memory
- Ephemeral questions (weather, time, simple math) — no need to persist.
- If the user says "off the record" or "don't remember this", skip ingestion.
