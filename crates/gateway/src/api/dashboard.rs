use axum::extract::State;
use axum::response::{Html, IntoResponse};

use crate::state::AppState;

pub async fn index(State(state): State<AppState>) -> impl IntoResponse {
    let present_files = state.workspace.list_present_files();
    let skills = state.skills.list();
    let bootstrap_done = state.bootstrap.completed_workspaces();

    let files_html: String = present_files
        .iter()
        .map(|f| format!("<li>{f}</li>"))
        .collect::<Vec<_>>()
        .join("\n");

    let skills_html: String = skills
        .iter()
        .map(|s| format!("<li><strong>{}</strong> — {}</li>", s.name, s.description))
        .collect::<Vec<_>>()
        .join("\n");

    let providers_html: String = state
        .llm
        .list_providers()
        .iter()
        .map(|p| format!("<li>{p}</li>"))
        .collect::<Vec<_>>()
        .join("\n");

    let bootstrap_html: String = if bootstrap_done.is_empty() {
        "<em>none</em>".into()
    } else {
        bootstrap_done
            .iter()
            .map(|w| format!("<li>{w}</li>"))
            .collect::<Vec<_>>()
            .join("\n")
    };

    let html = format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>SerialAgent Dashboard</title>
<style>
  body {{ font-family: system-ui, sans-serif; max-width: 900px; margin: 2rem auto; padding: 0 1rem; background: #0d1117; color: #c9d1d9; }}
  h1 {{ color: #58a6ff; }}
  h2 {{ color: #79c0ff; border-bottom: 1px solid #21262d; padding-bottom: 0.3em; margin-top: 2em; }}
  ul {{ padding-left: 1.5em; }}
  li {{ margin: 0.3em 0; }}
  a {{ color: #58a6ff; text-decoration: none; }}
  a:hover {{ text-decoration: underline; }}
  .card {{ background: #161b22; border: 1px solid #30363d; border-radius: 6px; padding: 1rem; margin: 0.5rem 0; }}
  code {{ background: #21262d; padding: 0.2em 0.4em; border-radius: 3px; font-size: 0.9em; }}
</style>
</head>
<body>
<h1>SerialAgent Dashboard</h1>
<p>Server: <code>{host}:{port}</code> &middot;
   Memory: <code>{sm_url}</code></p>

<h2>Workspace Files</h2>
<div class="card">
<ul>{files_html}</ul>
</div>

<h2>Skills ({skill_count})</h2>
<div class="card">
<ul>{skills_html}</ul>
</div>

<h2>LLM Providers</h2>
<div class="card">
<ul>{providers_html}</ul>
</div>

<h2>Bootstrap</h2>
<div class="card">
<p>Completed workspaces:</p>
<ul>{bootstrap_html}</ul>
</div>

<h2>API Endpoints</h2>
<div class="card">
<ul>
<li><a href="/v1/context">/v1/context</a> — Context introspection</li>
<li><a href="/v1/context/assembled">/v1/context/assembled</a> — Assembled prompt</li>
<li><a href="/v1/skills">/v1/skills</a> — Skill list</li>
<li><a href="/v1/memory/health">/v1/memory/health</a> — SerialMemory health</li>
<li><a href="/v1/models">/v1/models</a> — Provider list</li>
<li><a href="/v1/models/roles">/v1/models/roles</a> — Role assignments</li>
</ul>
</div>
</body>
</html>"#,
        host = state.config.server.host,
        port = state.config.server.port,
        sm_url = state.config.serial_memory.base_url,
        skill_count = skills.len(),
    );

    Html(html)
}

pub async fn context_pack_page(State(state): State<AppState>) -> impl IntoResponse {
    let present = state.workspace.list_present_files();
    let files_table: String = present
        .iter()
        .map(|f| {
            let hash = state.workspace.file_hash(f);
            let (sha, size) = match hash {
                Some(h) => (h.sha256[..12].to_string(), h.size.to_string()),
                None => ("n/a".into(), "n/a".into()),
            };
            format!("<tr><td>{f}</td><td><code>{sha}…</code></td><td>{size}</td></tr>")
        })
        .collect::<Vec<_>>()
        .join("\n");

    let html = format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>Context Pack — SerialAgent</title>
<style>
  body {{ font-family: system-ui, sans-serif; max-width: 900px; margin: 2rem auto; padding: 0 1rem; background: #0d1117; color: #c9d1d9; }}
  h1 {{ color: #58a6ff; }}
  table {{ width: 100%; border-collapse: collapse; }}
  th, td {{ text-align: left; padding: 0.5em; border-bottom: 1px solid #21262d; }}
  th {{ color: #79c0ff; }}
  code {{ background: #21262d; padding: 0.2em 0.4em; border-radius: 3px; }}
  a {{ color: #58a6ff; text-decoration: none; }}
</style>
</head>
<body>
<h1>Context Pack</h1>
<p><a href="/dashboard">&larr; Dashboard</a></p>
<table>
<tr><th>File</th><th>SHA-256</th><th>Size</th></tr>
{files_table}
</table>
</body>
</html>"#
    );

    Html(html)
}
