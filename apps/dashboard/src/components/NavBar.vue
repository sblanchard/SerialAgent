<script setup lang="ts">
import { useRoute } from "vue-router";
const route = useRoute();

const links = [
  { to: "/", label: "Overview", icon: "~" },
  { to: "/inbox", label: "Inbox", icon: "=" },
  { to: "/chat", label: "Chat", icon: ">" },
  { to: "/schedules", label: "Schedules", icon: "&" },
  { to: "/runs", label: "Runs", icon: "!" },
  { to: "/skills", label: "Skills", icon: "^" },
  { to: "/usage", label: "Usage", icon: "$" },
  { to: "/logs", label: "Logs", icon: "?" },
  { to: "/nodes", label: "Nodes", icon: "." },
  { to: "/agents", label: "Agents", icon: "@" },
  { to: "/sessions", label: "Sessions", icon: "#" },
  { to: "/llm", label: "LLM", icon: "*" },
  { to: "/import", label: "Import", icon: "+" },
  { to: "/staging", label: "Staging", icon: "%" },
  { to: "/settings", label: "Settings", icon: ":" },
];
</script>

<template>
  <nav class="nav">
    <div class="logo">SA</div>
    <ul>
      <li v-for="link in links" :key="link.to">
        <router-link
          :to="link.to"
          :class="{ active: route.path === link.to || (link.to !== '/' && route.path.startsWith(link.to)) }"
        >
          <span class="icon">{{ link.icon }}</span>
          <span class="label">{{ link.label }}</span>
        </router-link>
      </li>
    </ul>
  </nav>
</template>

<style scoped>
.nav {
  width: 200px;
  min-height: 100vh;
  background: var(--bg-card);
  border-right: 1px solid var(--border);
  padding: 1rem 0;
  flex-shrink: 0;
}
.logo {
  font-size: 1.4rem;
  font-weight: 700;
  color: var(--accent);
  padding: 0.5rem 1.2rem 1.5rem;
  letter-spacing: 0.05em;
}
ul { list-style: none; }
li a {
  display: flex;
  align-items: center;
  gap: 0.6rem;
  padding: 0.55rem 1.2rem;
  color: var(--text-dim);
  text-decoration: none;
  font-size: 0.9rem;
  transition: background 0.15s, color 0.15s;
}
li a:hover { background: rgba(88, 166, 255, 0.08); color: var(--text); }
li a.active {
  background: rgba(88, 166, 255, 0.12);
  color: var(--accent);
  border-left: 2px solid var(--accent);
}
.icon {
  font-family: var(--mono);
  font-weight: 700;
  width: 1.2em;
  text-align: center;
}
</style>
