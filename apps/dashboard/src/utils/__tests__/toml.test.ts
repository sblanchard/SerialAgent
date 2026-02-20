import { describe, it, expect, vi } from "vitest";
import { configToToml, downloadAsFile } from "../toml";
import type { SystemInfo, ReadinessResponse } from "@/api/client";

const mockInfo: SystemInfo = {
  version: "0.1.0",
  server: { host: "127.0.0.1", port: 3210 },
  admin_token_set: false,
  workspace_path: "workspace/",
  skills_path: "skills/",
  serial_memory_url: "http://localhost:8787",
  serial_memory_transport: "http",
  provider_count: 1,
  node_count: 0,
  session_count: 0,
};

const mockReadiness: ReadinessResponse = {
  ready: true,
  provider_count: 1,
  startup_policy: "lenient",
  providers: [
    {
      id: "openai/gpt-4o",
      capabilities: {
        supports_tools: "native",
        supports_streaming: true,
        supports_json_mode: true,
        supports_vision: true,
        context_window_tokens: 128000,
      },
    },
  ],
  init_errors: [],
  roles: { executor: "openai/gpt-4o" },
  has_executor: true,
  memory_configured: true,
  nodes_connected: 0,
};

describe("configToToml", () => {
  it("generates [server] section with host and port", () => {
    const toml = configToToml(mockInfo, mockReadiness);
    expect(toml).toContain("[server]");
    expect(toml).toContain('host = "127.0.0.1"');
    expect(toml).toContain("port = 3210");
  });

  it("includes provider sections", () => {
    const toml = configToToml(mockInfo, mockReadiness);
    expect(toml).toContain('[providers."openai/gpt-4o"]');
    expect(toml).toContain('model = "openai/gpt-4o"');
  });

  it("includes role assignments", () => {
    const toml = configToToml(mockInfo, mockReadiness);
    expect(toml).toContain("[roles]");
    expect(toml).toContain('executor = "openai/gpt-4o"');
  });
});

describe("downloadAsFile", () => {
  it("creates a blob URL and triggers download", () => {
    const revokeURL = vi.fn();
    vi.stubGlobal("URL", {
      createObjectURL: vi.fn().mockReturnValue("blob:test"),
      revokeObjectURL: revokeURL,
    });

    const clickSpy = vi.fn();
    vi.spyOn(document, "createElement").mockReturnValue({
      click: clickSpy,
      href: "",
      download: "",
    } as unknown as HTMLAnchorElement);

    downloadAsFile("test content", "config.toml");
    expect(clickSpy).toHaveBeenCalled();
    expect(revokeURL).toHaveBeenCalledWith("blob:test");
  });
});
