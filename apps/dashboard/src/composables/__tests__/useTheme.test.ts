import { describe, it, expect, vi, beforeEach } from "vitest";
import { nextTick } from "vue";

// Must be imported fresh for each test
let useTheme: typeof import("../useTheme").useTheme;

beforeEach(async () => {
  localStorage.clear();
  document.documentElement.removeAttribute("data-theme");
  vi.stubGlobal("matchMedia", vi.fn().mockReturnValue({ matches: false }));

  // Re-import to reset module-scoped state
  vi.resetModules();
  const mod = await import("../useTheme");
  useTheme = mod.useTheme;
});

describe("useTheme", () => {
  it("defaults to dark when no preference and prefers-color-scheme is dark", () => {
    const { theme } = useTheme();
    expect(theme.value).toBe("dark");
  });

  it("defaults to light when prefers-color-scheme is light", async () => {
    vi.stubGlobal("matchMedia", vi.fn().mockReturnValue({ matches: true }));
    vi.resetModules();
    const mod = await import("../useTheme");
    const { theme } = mod.useTheme();
    expect(theme.value).toBe("light");
  });

  it("reads stored preference from localStorage", async () => {
    localStorage.setItem("sa_theme", "light");
    vi.resetModules();
    const mod = await import("../useTheme");
    const { theme } = mod.useTheme();
    expect(theme.value).toBe("light");
  });

  it("toggleTheme switches and persists", async () => {
    const { theme, toggleTheme } = useTheme();
    expect(theme.value).toBe("dark");
    toggleTheme();
    await nextTick();
    expect(theme.value).toBe("light");
    expect(localStorage.getItem("sa_theme")).toBe("light");
  });
});
