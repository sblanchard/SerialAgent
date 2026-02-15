# First-Run Onboarding

This file is injected ONLY on the first session for a workspace.
After successful completion, bootstrap is marked done and this file
is not injected again.

## Onboarding Tasks

1. **Verify SerialMemory connection**: Call `/admin/status` and confirm the API is reachable.
2. **Initialize workspace in SerialMemory**: If using a dedicated tenant, call `workspace_create`.
3. **Load user profile**: Call `memory_about_user` to check if the user has existing data.
4. **Introduce yourself**: Explain that you have persistent memory and what that means.
5. **Ask for preferences**: Prompt the user for:
   - Preferred programming languages
   - Communication style (verbose vs concise)
   - Project context they want you to remember
6. **Store initial preferences**: Ingest the user's answers as `memory_type: knowledge`.
7. **Mark bootstrap complete**: The system will automatically mark this workspace as bootstrapped.
