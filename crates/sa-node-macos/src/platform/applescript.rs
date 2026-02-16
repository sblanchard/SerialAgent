//! AppleScript execution helper.
//!
//! Runs AppleScript via `osascript -e` and captures stdout.  This is the
//! simplest reference implementation for macOS automation.  For production
//! you'd use ScriptingBridge or Swift helpers for better performance and
//! error handling.

use std::io::Write;
use std::process::{Command, Stdio};

/// Execute an AppleScript snippet and return its stdout.
///
/// # Errors
///
/// Returns an error if `osascript` is not found, the script fails,
/// or the output contains non-UTF-8 bytes.
#[allow(dead_code)]
pub fn run(script: &str) -> Result<String, String> {
    let output = Command::new("osascript")
        .arg("-e")
        .arg(script)
        .output()
        .map_err(|e| format!("failed to run osascript: {e}"))?;

    classify_output(output)
}

/// Execute an AppleScript snippet, piping `stdin_data` to the process on
/// stdin.  The script can read it via `do shell script "cat /dev/stdin"`.
///
/// This is the preferred way to pass untrusted strings (e.g. user search
/// queries) into AppleScript — it avoids all string-interpolation injection
/// vectors.
pub fn run_with_stdin(script: &str, stdin_data: &str) -> Result<String, String> {
    let mut child = Command::new("osascript")
        .arg("-e")
        .arg(script)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to spawn osascript: {e}"))?;

    // Write stdin_data and close the pipe.
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(stdin_data.as_bytes())
            .map_err(|e| format!("failed to write to osascript stdin: {e}"))?;
    }

    let output = child
        .wait_with_output()
        .map_err(|e| format!("failed to wait on osascript: {e}"))?;

    classify_output(output)
}

/// Common output classification for both `run` and `run_with_stdin`.
fn classify_output(output: std::process::Output) -> Result<String, String> {
    let stderr = String::from_utf8_lossy(&output.stderr);

    if !output.status.success() {
        // Detect macOS TCC / Automation permission denial.
        let stderr_lower = stderr.to_ascii_lowercase();
        if stderr_lower.contains("not allowed assistive access")
            || stderr_lower.contains("not authorized to send apple events")
            || stderr_lower.contains("application isn't running")
            || stderr_lower.contains("erraeventnotpermitted")
            || stderr_lower.contains("-1743")
        {
            return Err(format!(
                "automation_denied: {}. \
                 Fix: open System Settings → Privacy & Security → Automation, \
                 and allow this app to control the target application.",
                stderr.trim()
            ));
        }

        return Err(format!(
            "osascript exited with {}: {}",
            output.status,
            stderr.trim()
        ));
    }

    String::from_utf8(output.stdout)
        .map(|s| s.trim().to_string())
        .map_err(|e| format!("non-UTF-8 output: {e}"))
}
