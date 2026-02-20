import { createRouter, createWebHashHistory } from "vue-router";

export const router = createRouter({
  history: createWebHashHistory(),
  routes: [
    { path: "/", name: "overview", component: () => import("./pages/Overview.vue") },
    { path: "/inbox", name: "inbox", component: () => import("./pages/Inbox.vue") },
    { path: "/chat", name: "chat", component: () => import("./pages/Chat.vue") },
    { path: "/schedules", name: "schedules", component: () => import("./pages/Schedules.vue") },
    { path: "/schedules/:id", name: "schedule-detail", component: () => import("./pages/ScheduleDetail.vue"), props: true },
    { path: "/runs", name: "runs", component: () => import("./pages/Runs.vue") },
    { path: "/runs/:id", name: "run-detail", component: () => import("./pages/RunDetail.vue"), props: true },
    { path: "/skills", name: "skills-engine", component: () => import("./pages/SkillsEngine.vue") },
    { path: "/usage", name: "usage", component: () => import("./pages/Usage.vue") },
    { path: "/logs", name: "logs", component: () => import("./pages/Logs.vue") },
    { path: "/nodes", name: "nodes", component: () => import("./pages/Nodes.vue") },
    { path: "/agents", name: "agents", component: () => import("./pages/Agents.vue") },
    { path: "/sessions", name: "sessions", component: () => import("./pages/Sessions.vue") },
    { path: "/sessions/:key", name: "session-detail", component: () => import("./pages/SessionDetail.vue"), props: true },
    { path: "/llm", name: "llm-readiness", component: () => import("./pages/LlmReadiness.vue") },
    { path: "/import", name: "import-openclaw", component: () => import("./pages/ImportOpenClaw.vue") },
    { path: "/staging", name: "staging", component: () => import("./pages/Staging.vue") },
    { path: "/settings", name: "settings", component: () => import("./pages/Settings.vue") },
  ],
});
