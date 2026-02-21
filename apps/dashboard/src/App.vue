<script setup lang="ts">
import NavBar from "./components/NavBar.vue";
import { useTheme } from "./composables/useTheme";

useTheme(); // Apply data-theme attribute on mount
</script>

<template>
  <div class="app">
    <NavBar />
    <main class="main">
      <router-view v-slot="{ Component }">
        <keep-alive include="ChatPage">
          <component :is="Component" />
        </keep-alive>
      </router-view>
    </main>
  </div>
</template>

<style>
* { margin: 0; padding: 0; box-sizing: border-box; }

:root {
  --bg: #0d1117;
  --bg-card: #161b22;
  --bg-code: #21262d;
  --border: #30363d;
  --text: #c9d1d9;
  --text-dim: #8b949e;
  --accent: #58a6ff;
  --accent-dim: #388bfd;
  --green: #3fb950;
  --red: #f85149;
  --yellow: #d29922;
  --font: system-ui, -apple-system, sans-serif;
  --mono: "SF Mono", "Cascadia Code", "Fira Code", monospace;
}

[data-theme="light"] {
  --bg: #ffffff;
  --bg-card: #f6f8fa;
  --bg-code: #e6edf3;
  --border: #d0d7de;
  --text: #1f2328;
  --text-dim: #656d76;
  --accent: #0969da;
  --accent-dim: #0550ae;
  --green: #1a7f37;
  --red: #cf222e;
  --yellow: #9a6700;
}

body {
  font-family: var(--font);
  background: var(--bg);
  color: var(--text);
  min-height: 100vh;
}
.app { display: flex; min-height: 100vh; }
.main { flex: 1; padding: 2rem; max-width: 1200px; overflow-x: auto; }
a { color: var(--accent); text-decoration: none; }
a:hover { text-decoration: underline; }
code {
  font-family: var(--mono);
  background: var(--bg-code);
  padding: 0.15em 0.4em;
  border-radius: 3px;
  font-size: 0.88em;
}
</style>
