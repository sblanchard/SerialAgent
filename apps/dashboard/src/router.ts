import { createRouter, createWebHashHistory } from "vue-router";

import Overview from "./pages/Overview.vue";
import Nodes from "./pages/Nodes.vue";
import Agents from "./pages/Agents.vue";
import Sessions from "./pages/Sessions.vue";
import SessionDetail from "./pages/SessionDetail.vue";
import LlmReadiness from "./pages/LlmReadiness.vue";
import ImportOpenClaw from "./pages/ImportOpenClaw.vue";
import Staging from "./pages/Staging.vue";
import Runs from "./pages/Runs.vue";
import RunDetail from "./pages/RunDetail.vue";
import Inbox from "./pages/Inbox.vue";
import Schedules from "./pages/Schedules.vue";
import SkillsEngine from "./pages/SkillsEngine.vue";
import Usage from "./pages/Usage.vue";
import Chat from "./pages/Chat.vue";
import Logs from "./pages/Logs.vue";
import Settings from "./pages/Settings.vue";

export const router = createRouter({
  history: createWebHashHistory(),
  routes: [
    { path: "/", name: "overview", component: Overview },
    { path: "/inbox", name: "inbox", component: Inbox },
    { path: "/chat", name: "chat", component: Chat },
    { path: "/schedules", name: "schedules", component: Schedules },
    { path: "/runs", name: "runs", component: Runs },
    { path: "/runs/:id", name: "run-detail", component: RunDetail, props: true },
    { path: "/skills", name: "skills-engine", component: SkillsEngine },
    { path: "/usage", name: "usage", component: Usage },
    { path: "/logs", name: "logs", component: Logs },
    { path: "/nodes", name: "nodes", component: Nodes },
    { path: "/agents", name: "agents", component: Agents },
    { path: "/sessions", name: "sessions", component: Sessions },
    { path: "/sessions/:key", name: "session-detail", component: SessionDetail, props: true },
    { path: "/llm", name: "llm-readiness", component: LlmReadiness },
    { path: "/import", name: "import-openclaw", component: ImportOpenClaw },
    { path: "/staging", name: "staging", component: Staging },
    { path: "/settings", name: "settings", component: Settings },
  ],
});
