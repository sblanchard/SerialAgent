import { ref, watchEffect } from "vue";

export type Theme = "dark" | "light";

const STORAGE_KEY = "sa_theme";

function getInitialTheme(): Theme {
  const stored = localStorage.getItem(STORAGE_KEY);
  if (stored === "light" || stored === "dark") return stored;
  return window.matchMedia("(prefers-color-scheme: light)").matches
    ? "light"
    : "dark";
}

// Module-scoped singleton — all callers share the same reactive state.
const theme = ref<Theme>(getInitialTheme());

export function useTheme() {
  watchEffect(() => {
    document.documentElement.setAttribute("data-theme", theme.value);
    localStorage.setItem(STORAGE_KEY, theme.value);
  });

  function toggleTheme(): void {
    theme.value = theme.value === "dark" ? "light" : "dark";
  }

  return { theme, toggleTheme } as const;
}
