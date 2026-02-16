import { createRouter, createWebHashHistory } from "vue-router";

import Overview from "./pages/Overview.vue";
import Nodes from "./pages/Nodes.vue";
import Agents from "./pages/Agents.vue";
import Sessions from "./pages/Sessions.vue";
import SessionDetail from "./pages/SessionDetail.vue";

export const router = createRouter({
  history: createWebHashHistory(),
  routes: [
    { path: "/", name: "overview", component: Overview },
    { path: "/nodes", name: "nodes", component: Nodes },
    { path: "/agents", name: "agents", component: Agents },
    { path: "/sessions", name: "sessions", component: Sessions },
    { path: "/sessions/:key", name: "session-detail", component: SessionDetail, props: true },
  ],
});
