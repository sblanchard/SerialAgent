//! `serialagent run` — one-shot execution command.
//!
//! Sends a single message to the agent, streams the response to stdout,
//! and exits.  Useful for scripting, piping, and quick CLI interactions.

use std::io::Write;
use std::sync::Arc;

use sa_domain::config::Config;
use sa_sessions::store::SessionOrigin;

use crate::bootstrap;
use crate::runtime::{run_turn, TurnEvent, TurnInput};

/// Execute a single agent turn and print the response.
///
/// This is the entry point for `serialagent run "message"`.
pub async fn run(
    config: Arc<Config>,
    message: String,
    session_key: String,
    model: Option<String>,
    json_output: bool,
) -> anyhow::Result<()> {
    // 1. Boot the full runtime (without background tasks).
    let state = bootstrap::build_app_state(config).await?;

    // 2. Resolve or create the session.
    let (entry, _is_new) = state
        .sessions
        .resolve_or_create(&session_key, SessionOrigin::default());

    // 3. Build the turn input.
    let input = TurnInput {
        session_key: session_key.clone(),
        session_id: entry.session_id.clone(),
        user_message: message,
        model,
        response_format: None,
        agent: None,
    };

    // 4. Run the turn and obtain the event receiver.
    let (_run_id, mut rx) = run_turn(state.clone(), input);

    // 5. Drain the receiver, printing events to stdout/stderr.
    let mut exit_code: i32 = 0;
    let mut collected_events: Vec<TurnEvent> = Vec::new();

    while let Some(event) = rx.recv().await {
        if json_output {
            collected_events.push(event);
        } else {
            match &event {
                TurnEvent::AssistantDelta { text } => {
                    print!("{text}");
                    std::io::stdout().flush().ok();
                }
                TurnEvent::Thought { content } => {
                    // Dim output to stderr so it doesn't pollute stdout.
                    eprint!("\x1b[2m{content}\x1b[0m");
                    std::io::stderr().flush().ok();
                }
                TurnEvent::ToolCallEvent { tool_name, .. } => {
                    eprintln!("\x1b[2m[tool: {tool_name}]\x1b[0m");
                }
                TurnEvent::Final { .. } => {
                    // Ensure a trailing newline after streamed deltas.
                    println!();
                }
                TurnEvent::Error { message } => {
                    eprintln!("error: {message}");
                    exit_code = 1;
                }
                TurnEvent::Stopped { .. } => {
                    eprintln!("turn stopped");
                }
                _ => {}
            }
        }
    }

    // 6. In JSON mode, serialize all collected events to stdout.
    if json_output {
        let json = serde_json::to_string_pretty(&collected_events)
            .map_err(|e| anyhow::anyhow!("serializing events: {e}"))?;
        println!("{json}");
    }

    // 7. Flush session store before exit.
    if let Err(e) = state.sessions.flush().await {
        tracing::warn!(error = %e, "session store flush on exit failed");
    }

    if exit_code != 0 {
        std::process::exit(exit_code);
    }

    Ok(())
}
