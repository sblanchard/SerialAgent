import type { SystemInfo, ReadinessResponse } from "@/api/client";

/**
 * Generate a TOML config template from the running system's info and
 * readiness response.  This is an approximation — only fields visible
 * through the admin API are included.
 */
export function configToToml(
  info: SystemInfo,
  readiness: ReadinessResponse,
): string {
  const lines: string[] = [
    "# SerialAgent configuration",
    "# Generated from running system — review and adjust before use.",
    "",
    "[server]",
    `host = "${info.server.host}"`,
    `port = ${info.server.port}`,
    "",
    "# cors.allowed_origins = [\"http://localhost:*\"]",
    "# api_token_env = \"SA_API_TOKEN\"",
    "",
  ];

  // Providers
  if (readiness.providers.length > 0) {
    for (const p of readiness.providers) {
      lines.push(`[providers.${sanitizeKey(p.id)}]`);

      // Try to infer provider type from the id
      const model = inferModelLine(p.id);
      if (model) lines.push(model);

      lines.push(
        `# context_window = ${p.capabilities.context_window_tokens}`,
        `# supports_tools = ${p.capabilities.supports_tools}`,
        `# supports_streaming = ${p.capabilities.supports_streaming}`,
        "",
      );
    }
  }

  // Role assignments
  if (Object.keys(readiness.roles).length > 0) {
    lines.push("[roles]");
    for (const [role, provider] of Object.entries(readiness.roles)) {
      lines.push(`${role} = "${provider}"`);
    }
    lines.push("");
  }

  // Memory
  if (info.serial_memory_url) {
    lines.push("[memory]");
    lines.push(`url = "${info.serial_memory_url}"`);
    if (info.serial_memory_transport) {
      lines.push(`transport = "${info.serial_memory_transport}"`);
    }
    lines.push("");
  }

  // Workspace / skills
  lines.push("# [workspace]");
  lines.push(`# path = "${info.workspace_path}"`);
  lines.push("");
  lines.push("# [skills]");
  lines.push(`# path = "${info.skills_path}"`);
  lines.push("");

  return lines.join("\n");
}

/** Sanitize a provider ID for use as a TOML key (quote if needed). */
function sanitizeKey(id: string): string {
  return /^[a-zA-Z0-9_-]+$/.test(id) ? id : `"${id}"`;
}

/** Infer a model line from a provider id like "openai/gpt-4o". */
function inferModelLine(id: string): string | null {
  if (id.includes("/")) {
    return `model = "${id}"`;
  }
  return `# model = "${id}"`;
}

/** Trigger a file download in the browser. */
export function downloadAsFile(content: string, filename: string): void {
  const blob = new Blob([content], { type: "text/plain" });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = filename;
  a.click();
  URL.revokeObjectURL(url);
}
