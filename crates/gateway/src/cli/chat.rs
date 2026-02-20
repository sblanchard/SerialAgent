//! `serialagent chat` — interactive REPL command.
//!
//! Opens a readline-based loop that sends each line to the agent and
//! streams the response back.  Supports slash-commands for session
//! management, model switching, and other REPL conveniences.

use std::io::Write;
use std::sync::Arc;

use sa_domain::config::Config;
use sa_sessions::store::SessionOrigin;

use crate::bootstrap;
use crate::runtime::{run_turn, TurnEvent, TurnInput};

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Public entry point
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Run the interactive chat REPL.
///
/// Boots the full runtime (including background tasks for session
/// flushing), then enters a readline loop that accepts user input and
/// streams agent responses to stdout.
pub async fn chat(
    config: Arc<Config>,
    mut session_key: String,
    mut model: Option<String>,
) -> anyhow::Result<()> {
    // 1. Boot the full runtime.
    let state = bootstrap::build_app_state(config).await?;

    // 2. Spawn background tasks (chat is long-lived).
    bootstrap::spawn_background_tasks(&state);

    // 3. Initialize rustyline editor with persistent history.
    let history_path = dirs::home_dir()
        .unwrap_or_default()
        .join(".serialagent")
        .join("chat_history.txt");
    if let Some(parent) = history_path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let mut rl = rustyline::DefaultEditor::new()?;
    let _ = rl.load_history(&history_path);

    // 4. Print welcome message to stderr (keep stdout clean for output).
    eprintln!("SerialAgent interactive chat");
    eprintln!(
        "Session: {session_key}  |  Type /help for commands, Ctrl+D to exit"
    );
    eprintln!();

    // 5. REPL loop.
    loop {
        let readline = rl.readline("you> ");

        match readline {
            Ok(line) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }

                rl.add_history_entry(&line).ok();

                // ── Slash commands ────────────────────────────────
                if trimmed.starts_with('/') {
                    if handle_slash_command(
                        trimmed,
                        &mut session_key,
                        &mut model,
                    ) {
                        break;
                    }
                    continue;
                }

                // ── User message → agent turn ────────────────────
                if let Err(e) =
                    send_message(&state, &session_key, &model, trimmed).await
                {
                    eprintln!("\x1B[31merror: {e}\x1B[0m");
                }
            }
            Err(rustyline::error::ReadlineError::Interrupted) => {
                eprintln!("(Use Ctrl+D or /exit to quit)");
                continue;
            }
            Err(rustyline::error::ReadlineError::Eof) => {
                break;
            }
            Err(e) => {
                eprintln!("\x1B[31mreadline error: {e}\x1B[0m");
                break;
            }
        }
    }

    // 6. Save history.
    rl.save_history(&history_path).ok();

    // 7. Flush sessions before exit.
    state.sessions.flush().await.ok();

    eprintln!("Goodbye!");
    Ok(())
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Slash command handling
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Process a slash command.  Returns `true` if the REPL should exit.
fn handle_slash_command(
    input: &str,
    session_key: &mut String,
    model: &mut Option<String>,
) -> bool {
    let parts: Vec<&str> = input.splitn(2, ' ').collect();
    let cmd = parts[0];
    let arg = parts.get(1).map(|s| s.trim());

    match cmd {
        "/exit" | "/quit" => return true,

        "/session" => {
            if let Some(name) = arg.filter(|s| !s.is_empty()) {
                *session_key = name.to_string();
                eprintln!("Session switched to: {session_key}");
            } else {
                eprintln!("Current session: {session_key}");
                eprintln!("Usage: /session <name>");
            }
        }

        "/model" => {
            if let Some(name) = arg.filter(|s| !s.is_empty()) {
                *model = Some(name.to_string());
                eprintln!("Model set to: {name}");
            } else {
                let current = model
                    .as_deref()
                    .unwrap_or("(default)");
                eprintln!("Current model: {current}");
                eprintln!("Usage: /model <name>");
            }
        }

        "/clear" => {
            // ANSI escape: clear screen and move cursor to top-left.
            eprint!("\x1B[2J\x1B[1;1H");
        }

        "/reset" => {
            let ts = chrono::Utc::now().timestamp();
            *session_key = format!("{session_key}:{ts}");
            eprintln!("Session reset. New session key: {session_key}");
        }

        "/help" => {
            eprintln!("Commands:");
            eprintln!("  /session <name>  Switch to a named session");
            eprintln!("  /model <name>    Set the model (e.g. openai/gpt-4o)");
            eprintln!("  /clear           Clear the screen");
            eprintln!("  /reset           Start a fresh session (new key)");
            eprintln!("  /exit, /quit     Exit the chat");
            eprintln!("  /help            Show this help");
        }

        other => {
            eprintln!("Unknown command: {other}  (type /help for a list)");
        }
    }

    false
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Message sending + event streaming
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Resolve the session, build a [`TurnInput`], call [`run_turn`], and
/// stream events to stdout/stderr.
async fn send_message(
    state: &crate::state::AppState,
    session_key: &str,
    model: &Option<String>,
    user_message: &str,
) -> anyhow::Result<()> {
    // Resolve or create the session.
    let (entry, _is_new) = state
        .sessions
        .resolve_or_create(session_key, SessionOrigin::default());

    let input = TurnInput {
        session_key: session_key.to_string(),
        session_id: entry.session_id.clone(),
        user_message: user_message.to_string(),
        model: model.clone(),
        agent: None,
    };

    let (_run_id, mut rx) = run_turn(state.clone(), input);

    // Stream events.
    while let Some(event) = rx.recv().await {
        match &event {
            TurnEvent::AssistantDelta { text } => {
                print!("{text}");
                std::io::stdout().flush().ok();
            }
            TurnEvent::Thought { content } => {
                eprint!("\x1B[2m{content}\x1B[0m");
                std::io::stderr().flush().ok();
            }
            TurnEvent::ToolCallEvent { tool_name, .. } => {
                eprintln!("\x1B[2m[tool: {tool_name}]\x1B[0m");
            }
            TurnEvent::Final { .. } => {
                // Ensure trailing newline + blank separator after response.
                println!();
                println!();
            }
            TurnEvent::Error { message } => {
                eprintln!("\x1B[31merror: {message}\x1B[0m");
            }
            TurnEvent::Stopped { .. } => {
                eprintln!("(turn stopped)");
            }
            _ => {}
        }
    }

    Ok(())
}
