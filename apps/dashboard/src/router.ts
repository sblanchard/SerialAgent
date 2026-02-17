import { createRouter, createWebHashHistory } from "vue-router";

import Overview from "./pages/Overview.vue";
import Nodes from "./pages/Nodes.vue";
import Agents from "./pages/Agents.vue";
import Sessions from "./pages/Sessions.vue";
import SessionDetail from "./pages/SessionDetail.vue";
import LlmReadiness from "./pages/LlmReadiness.vue";
import ImportOpenClaw from "./pages/ImportOpenClaw.vue";

export const router = createRouter({
  history: createWebHashHistory(),
  routes: [
    { path: "/", name: "overview", component: Overview },
    { path: "/nodes", name: "nodes", component: Nodes },
    { path: "/agents", name: "agents", component: Agents },
    { path: "/sessions", name: "sessions", component: Sessions },
    { path: "/sessions/:key", name: "session-detail", component: SessionDetail, props: true },
    { path: "/llm", name: "llm-readiness", component: LlmReadiness },
    { path: "/import", name: "import-openclaw", component: ImportOpenClaw },
  ],
});
