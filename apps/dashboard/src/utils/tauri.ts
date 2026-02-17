/**
 * Tauri environment detection and IPC helpers.
 *
 * When the dashboard runs inside the Tauri desktop shell,
 * `window.__TAURI_INTERNALS__` is available. In a plain browser
 * this is undefined, so every call gracefully falls back.
 */

/** True when the app is running inside the Tauri webview. */
export const isTauri: boolean =
  typeof window !== "undefined" &&
  "__TAURI_INTERNALS__" in window;
