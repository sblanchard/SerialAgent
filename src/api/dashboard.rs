use axum::extract::State;
use axum::response::{Html, IntoResponse};

use crate::context::builder::ContextPackBuilder;
use crate::memory::user_facts::UserFactsBuilder;
use crate::AppState;

/// GET /dashboard
///
/// Main dashboard page with navigation.
pub async fn index(State(_state): State<AppState>) -> impl IntoResponse {
    Html(DASHBOARD_HTML.to_string())
}

/// GET /dashboard/context
///
/// Context Pack debug page — shows files, sizes, truncation flags, and assembled prompt.
pub async fn context_pack_page(State(state): State<AppState>) -> impl IntoResponse {
    let workspace_id = "default";
    let is_first_run = state.bootstrap.is_first_run(workspace_id);

    let facts_builder = UserFactsBuilder::new(
        state.memory_client.clone(),
        state.config.clone(),
    );
    let user_facts = facts_builder
        .build(&state.config.serial_memory.default_user_id)
        .await
        .ok();

    let builder = ContextPackBuilder::new(
        state.config.clone(),
        state.workspace.clone(),
        state.skills.clone(),
    );

    let (assembled, report) = match builder.build(is_first_run, user_facts.as_deref()) {
        Ok(result) => result,
        Err(e) => {
            return Html(format!(
                "<html><body><h1>Context Build Error</h1><pre>{e}</pre></body></html>"
            ))
        }
    };

    // Build the table rows
    let mut rows = String::new();
    for file in &report.files {
        let trunc_reason = if file.truncated_total_cap {
            "total-cap"
        } else if file.truncated_per_file {
            "per-file"
        } else {
            "—"
        };

        let status = if file.included { "included" } else { "excluded" };
        let status_class = if file.included { "included" } else { "excluded" };

        rows.push_str(&format!(
            r#"<tr>
                <td><code>{}</code></td>
                <td class="num">{}</td>
                <td class="num">{}</td>
                <td>{}</td>
                <td class="{}">{}</td>
            </tr>"#,
            file.filename, file.raw_chars, file.injected_chars, trunc_reason, status_class, status,
        ));
    }

    let present_files = state.workspace.list_present_files();
    let skills_list = state.skills.list();
    let assembled_escaped = assembled
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");

    let html = format!(
        r##"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="utf-8">
    <title>SerialAssistant — Context Pack</title>
    <style>
        * {{ margin: 0; padding: 0; box-sizing: border-box; }}
        body {{ font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', system-ui, sans-serif;
                background: #0d1117; color: #c9d1d9; padding: 2rem; }}
        h1 {{ color: #58a6ff; margin-bottom: 0.5rem; font-size: 1.5rem; }}
        h2 {{ color: #8b949e; margin: 1.5rem 0 0.5rem; font-size: 1.1rem; }}
        .badge {{ display: inline-block; padding: 2px 8px; border-radius: 4px;
                   font-size: 0.75rem; font-weight: 600; }}
        .badge-ok {{ background: #238636; color: #fff; }}
        .badge-warn {{ background: #d29922; color: #000; }}
        table {{ width: 100%; border-collapse: collapse; margin-bottom: 1rem; }}
        th, td {{ text-align: left; padding: 8px 12px; border-bottom: 1px solid #21262d; }}
        th {{ color: #8b949e; font-size: 0.85rem; text-transform: uppercase; }}
        .num {{ text-align: right; font-variant-numeric: tabular-nums; }}
        .included {{ color: #3fb950; }}
        .excluded {{ color: #f85149; }}
        pre {{ background: #161b22; border: 1px solid #30363d; border-radius: 6px;
               padding: 1rem; overflow-x: auto; font-size: 0.85rem; max-height: 600px;
               overflow-y: auto; }}
        code {{ font-family: 'SF Mono', 'Fira Code', monospace; }}
        .stats {{ display: flex; gap: 2rem; margin: 1rem 0; }}
        .stat {{ background: #161b22; border: 1px solid #30363d; border-radius: 6px;
                 padding: 1rem; min-width: 150px; }}
        .stat-value {{ font-size: 1.5rem; font-weight: 700; color: #58a6ff; }}
        .stat-label {{ font-size: 0.8rem; color: #8b949e; margin-top: 4px; }}
        button {{ background: #21262d; color: #c9d1d9; border: 1px solid #30363d;
                  padding: 6px 16px; border-radius: 6px; cursor: pointer; font-size: 0.85rem; }}
        button:hover {{ background: #30363d; }}
        a {{ color: #58a6ff; text-decoration: none; }}
        a:hover {{ text-decoration: underline; }}
        nav {{ margin-bottom: 2rem; }}
        nav a {{ margin-right: 1rem; }}
    </style>
</head>
<body>
    <nav>
        <a href="/dashboard">Dashboard</a>
        <a href="/dashboard/context">Context Pack</a>
        <a href="/v1/skills">Skills API</a>
        <a href="/v1/context">Context API</a>
    </nav>

    <h1>Context Pack Inspector</h1>
    <p style="color:#8b949e; margin-bottom:1rem;">
        Workspace: <code>default</code>
        <span class="badge {first_run_class}">{first_run_label}</span>
    </p>

    <div class="stats">
        <div class="stat">
            <div class="stat-value">{total_injected}</div>
            <div class="stat-label">Total Injected Chars</div>
        </div>
        <div class="stat">
            <div class="stat-value">{files_found}</div>
            <div class="stat-label">Files Found</div>
        </div>
        <div class="stat">
            <div class="stat-value">{skills_count}</div>
            <div class="stat-label">Skills Registered</div>
        </div>
        <div class="stat">
            <div class="stat-value">{skills_chars}</div>
            <div class="stat-label">Skills Index Chars</div>
        </div>
        <div class="stat">
            <div class="stat-value">{facts_chars}</div>
            <div class="stat-label">User Facts Chars</div>
        </div>
    </div>

    <h2>Workspace Files</h2>
    <table>
        <thead>
            <tr>
                <th>File</th>
                <th class="num">Raw Chars</th>
                <th class="num">Injected Chars</th>
                <th>Truncation</th>
                <th>Status</th>
            </tr>
        </thead>
        <tbody>
            {rows}
        </tbody>
    </table>

    <h2>Assembled System Prompt</h2>
    <button onclick="navigator.clipboard.writeText(document.getElementById('assembled').textContent)">
        Copy to Clipboard
    </button>
    <pre id="assembled"><code>{assembled}</code></pre>
</body>
</html>"##,
        first_run_class = if is_first_run { "badge-warn" } else { "badge-ok" },
        first_run_label = if is_first_run { "FIRST RUN" } else { "BOOTSTRAPPED" },
        total_injected = report.total_injected_chars,
        files_found = present_files.len(),
        skills_count = skills_list.len(),
        skills_chars = report.skills_index_chars,
        facts_chars = report.user_facts_chars,
        rows = rows,
        assembled = assembled_escaped,
    );

    Html(html)
}

const DASHBOARD_HTML: &str = r##"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="utf-8">
    <title>SerialAssistant Dashboard</title>
    <style>
        * { margin: 0; padding: 0; box-sizing: border-box; }
        body { font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', system-ui, sans-serif;
               background: #0d1117; color: #c9d1d9; padding: 2rem; }
        h1 { color: #58a6ff; margin-bottom: 1rem; }
        .card { background: #161b22; border: 1px solid #30363d; border-radius: 8px;
                padding: 1.5rem; margin-bottom: 1rem; }
        .card h2 { color: #c9d1d9; font-size: 1.1rem; margin-bottom: 0.5rem; }
        .card p { color: #8b949e; font-size: 0.9rem; }
        a { color: #58a6ff; text-decoration: none; }
        a:hover { text-decoration: underline; }
        .grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(300px, 1fr)); gap: 1rem; }
    </style>
</head>
<body>
    <h1>SerialAssistant</h1>
    <p style="color:#8b949e; margin-bottom:2rem;">
        AI Assistant orchestrator with SerialMemory backend
    </p>

    <div class="grid">
        <div class="card">
            <h2><a href="/dashboard/context">Context Pack</a></h2>
            <p>Inspect workspace files, truncation, skills index, and the assembled system prompt.</p>
        </div>
        <div class="card">
            <h2><a href="/v1/skills">Skills Registry</a></h2>
            <p>View registered skills, their risk tiers, and load on-demand documentation.</p>
        </div>
        <div class="card">
            <h2><a href="/v1/context">Context API</a></h2>
            <p>Machine-readable context pack report (JSON).</p>
        </div>
        <div class="card">
            <h2><a href="/v1/memory/health">SerialMemory Health</a></h2>
            <p>Check connectivity to your SerialMemoryServer instance.</p>
        </div>
    </div>
</body>
</html>"##;
