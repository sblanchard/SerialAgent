//! AppleScript execution helper.
//!
//! Runs AppleScript via `osascript -e` and captures stdout.  This is the
//! simplest reference implementation for macOS automation.  For production
//! you'd use ScriptingBridge or Swift helpers for better performance and
//! error handling.

use std::process::Command;

/// Execute an AppleScript snippet and return its stdout.
///
/// # Errors
///
/// Returns an error if `osascript` is not found, the script fails,
/// or the output contains non-UTF-8 bytes.
pub fn run(script: &str) -> Result<String, String> {
    let output = Command::new("osascript")
        .arg("-e")
        .arg(script)
        .output()
        .map_err(|e| format!("failed to run osascript: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
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
