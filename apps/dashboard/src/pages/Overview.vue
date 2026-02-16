<script setup lang="ts">
import { ref, onMounted } from "vue";
import { api } from "@/api/client";
import type { ReadinessResponse, NodesListResponse, SessionsListResponse, AgentsListResponse } from "@/api/client";
import Card from "@/components/Card.vue";
import StatusDot from "@/components/StatusDot.vue";

const readiness = ref<ReadinessResponse | null>(null);
const nodes = ref<NodesListResponse | null>(null);
const sessions = ref<SessionsListResponse | null>(null);
const agents = ref<AgentsListResponse | null>(null);
const error = ref("");

async function load() {
  try {
    const [r, n, s, a] = await Promise.all([
      api.readiness(),
      api.nodes(),
      api.sessions(),
      api.agents(),
    ]);
    readiness.value = r;
    nodes.value = n;
    sessions.value = s;
    agents.value = a;
  } catch (e: any) {
    error.value = e.message;
  }
}

onMounted(load);
</script>

<template>
  <div>
    <h1 class="page-title">Overview</h1>
    <p v-if="error" class="error">{{ error }}</p>

    <div class="grid">
      <Card title="LLM Readiness">
        <template v-if="readiness">
          <p>
            <StatusDot :status="readiness.ready ? 'ok' : 'error'" />
            {{ readiness.ready ? "Ready" : "Not Ready" }}
          </p>
          <p class="dim">Policy: <code>{{ readiness.startup_policy }}</code></p>
          <p class="dim">Providers: {{ readiness.providers?.length ?? 0 }}</p>
          <ul v-if="readiness.errors?.length" class="errors">
            <li v-for="e in readiness.errors" :key="e">{{ e }}</li>
          </ul>
        </template>
        <p v-else class="dim">Loading...</p>
      </Card>

      <Card title="Connected Nodes">
        <template v-if="nodes">
          <p class="big-number">{{ nodes.count }}</p>
          <p class="dim">node{{ nodes.count !== 1 ? "s" : "" }} online</p>
        </template>
        <p v-else class="dim">Loading...</p>
      </Card>

      <Card title="Active Sessions">
        <template v-if="sessions">
          <p class="big-number">{{ sessions.count }}</p>
          <p class="dim">session{{ sessions.count !== 1 ? "s" : "" }} tracked</p>
        </template>
        <p v-else class="dim">Loading...</p>
      </Card>

      <Card title="Agents">
        <template v-if="agents">
          <p class="big-number">{{ agents.count }}</p>
          <p class="dim">agent{{ agents.count !== 1 ? "s" : "" }} configured</p>
        </template>
        <p v-else class="dim">Loading...</p>
      </Card>
    </div>

    <Card v-if="sessions && sessions.sessions.length > 0" title="Recently Active Sessions">
      <table class="tbl">
        <thead>
          <tr>
            <th>Session Key</th>
            <th>Channel</th>
            <th>Last Touched</th>
            <th>Tokens</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="s in sessions.sessions.slice(0, 10)" :key="s.session_key">
            <td>
              <router-link :to="{ name: 'session-detail', params: { key: s.session_key } }">
                <code>{{ s.session_key }}</code>
              </router-link>
            </td>
            <td>{{ s.origin?.channel || "-" }}</td>
            <td class="dim">{{ new Date(s.updated_at).toLocaleString() }}</td>
            <td class="dim">{{ s.total_tokens?.toLocaleString() ?? 0 }}</td>
          </tr>
        </tbody>
      </table>
    </Card>
  </div>
</template>

<style scoped>
.page-title { font-size: 1.5rem; margin-bottom: 1.5rem; color: var(--accent); }
.grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(220px, 1fr)); gap: 1rem; margin-bottom: 1.5rem; }
.big-number { font-size: 2.2rem; font-weight: 700; color: var(--text); }
.dim { color: var(--text-dim); font-size: 0.85rem; margin-top: 0.3rem; }
.error { color: var(--red); margin-bottom: 1rem; }
.errors { color: var(--red); padding-left: 1.2em; margin-top: 0.4em; font-size: 0.85rem; }
.tbl { width: 100%; border-collapse: collapse; font-size: 0.85rem; }
.tbl th { color: var(--text-dim); font-weight: 600; text-align: left; padding: 0.4rem 0.6rem; border-bottom: 1px solid var(--border); }
.tbl td { padding: 0.4rem 0.6rem; border-bottom: 1px solid var(--border); }
</style>
