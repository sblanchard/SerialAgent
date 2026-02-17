<script setup lang="ts">
import { ref, onMounted } from "vue";
import { api } from "@/api/client";
import type { ReadinessResponse, NodesListResponse, SessionsListResponse, AgentsListResponse, SystemInfo } from "@/api/client";
import Card from "@/components/Card.vue";
import StatusDot from "@/components/StatusDot.vue";

const readiness = ref<ReadinessResponse | null>(null);
const nodes = ref<NodesListResponse | null>(null);
const sessions = ref<SessionsListResponse | null>(null);
const agents = ref<AgentsListResponse | null>(null);
const sysInfo = ref<SystemInfo | null>(null);
const error = ref("");

async function load() {
  try {
    const [r, n, s, a, info] = await Promise.all([
      api.readiness(),
      api.nodes(),
      api.sessions(),
      api.agents(),
      api.systemInfo().catch(() => null),
    ]);
    readiness.value = r;
    nodes.value = n;
    sessions.value = s;
    agents.value = a;
    sysInfo.value = info;
  } catch (e: any) {
    error.value = e.message;
  }
}

function timeSince(iso: string): string {
  const ms = Date.now() - new Date(iso).getTime();
  const sec = Math.floor(ms / 1000);
  if (sec < 60) return `${sec}s ago`;
  const min = Math.floor(sec / 60);
  if (min < 60) return `${min}m ago`;
  const hr = Math.floor(min / 60);
  if (hr < 24) return `${hr}h ago`;
  return `${Math.floor(hr / 24)}d ago`;
}

onMounted(load);
</script>

<template>
  <div>
    <h1 class="page-title">Overview</h1>
    <p v-if="error" class="error">{{ error }}</p>

    <!-- System info bar -->
    <div v-if="sysInfo" class="sys-bar">
      <span>v{{ sysInfo.version }}</span>
      <span><code>{{ sysInfo.server.host }}:{{ sysInfo.server.port }}</code></span>
      <span>Memory: <code>{{ sysInfo.serial_memory_url }}</code></span>
      <span>
        <StatusDot :status="sysInfo.admin_token_set ? 'ok' : 'warn'" />
        Admin token: {{ sysInfo.admin_token_set ? "set" : "not set" }}
      </span>
    </div>

    <div class="grid">
      <Card title="LLM Readiness">
        <template v-if="readiness">
          <p>
            <StatusDot :status="readiness.ready ? 'ok' : 'error'" />
            {{ readiness.ready ? "Ready" : "Not Ready" }}
          </p>
          <p class="dim">Policy: <code>{{ readiness.startup_policy }}</code></p>
          <p class="dim">Providers: {{ readiness.provider_count }}</p>
          <p class="dim">
            Executor: <StatusDot :status="readiness.has_executor ? 'ok' : 'error'" />
            {{ readiness.has_executor ? "available" : "missing" }}
          </p>
          <div v-if="readiness.init_errors?.length" class="errors">
            <p v-for="e in readiness.init_errors" :key="e.provider_id" class="error-line">
              {{ e.provider_id }}: {{ e.error }}
            </p>
          </div>
          <router-link to="/llm" class="detail-link">Full details</router-link>
        </template>
        <p v-else class="dim">Loading...</p>
      </Card>

      <Card title="Connected Nodes">
        <template v-if="nodes">
          <p class="big-number">{{ nodes.count }}</p>
          <p class="dim">node{{ nodes.count !== 1 ? "s" : "" }} online</p>
          <router-link to="/nodes" class="detail-link">View nodes</router-link>
        </template>
        <p v-else class="dim">Loading...</p>
      </Card>

      <Card title="Active Sessions">
        <template v-if="sessions">
          <p class="big-number">{{ sessions.count }}</p>
          <p class="dim">session{{ sessions.count !== 1 ? "s" : "" }} tracked</p>
          <router-link to="/sessions" class="detail-link">View sessions</router-link>
        </template>
        <p v-else class="dim">Loading...</p>
      </Card>

      <Card title="Agents">
        <template v-if="agents">
          <p class="big-number">{{ agents.count }}</p>
          <p class="dim">agent{{ agents.count !== 1 ? "s" : "" }} configured</p>
          <router-link to="/agents" class="detail-link">View agents</router-link>
        </template>
        <p v-else class="dim">Loading...</p>
      </Card>
    </div>

    <!-- Per-channel breakdown -->
    <Card v-if="sessions && sessions.sessions.length > 0" title="Sessions by Channel">
      <div class="channel-grid">
        <div v-for="[ch, count] in channelCounts(sessions.sessions)" :key="ch" class="channel-item">
          <span class="channel-name">{{ ch || "(no channel)" }}</span>
          <span class="channel-count">{{ count }}</span>
        </div>
      </div>
    </Card>

    <Card v-if="sessions && sessions.sessions.length > 0" title="Recently Active Sessions">
      <table class="tbl">
        <thead>
          <tr>
            <th></th>
            <th>Session Key</th>
            <th>Channel</th>
            <th>Peer</th>
            <th>Last Touched</th>
            <th>Tokens</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="s in sessions.sessions.slice(0, 10)" :key="s.session_key">
            <td><StatusDot :status="s.running ? 'ok' : 'off'" /></td>
            <td>
              <router-link :to="{ name: 'session-detail', params: { key: s.session_key } }">
                <code>{{ s.session_key }}</code>
              </router-link>
            </td>
            <td>{{ s.origin?.channel || "-" }}</td>
            <td>{{ s.origin?.peer || "-" }}</td>
            <td class="dim">{{ timeSince(s.updated_at) }}</td>
            <td class="dim">{{ s.total_tokens?.toLocaleString() ?? 0 }}</td>
          </tr>
        </tbody>
      </table>
    </Card>
  </div>
</template>

<script lang="ts">
// Helper outside setup for template use
function channelCounts(sessions: any[]): [string, number][] {
  const map = new Map<string, number>();
  for (const s of sessions) {
    const ch = s.origin?.channel || "";
    map.set(ch, (map.get(ch) ?? 0) + 1);
  }
  return [...map.entries()].sort((a, b) => b[1] - a[1]);
}
</script>

<style scoped>
.page-title { font-size: 1.5rem; margin-bottom: 1.5rem; color: var(--accent); }
.grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(220px, 1fr)); gap: 1rem; margin-bottom: 1.5rem; }
.big-number { font-size: 2.2rem; font-weight: 700; color: var(--text); }
.dim { color: var(--text-dim); font-size: 0.85rem; margin-top: 0.3rem; }
.error { color: var(--red); margin-bottom: 1rem; }
.errors { margin-top: 0.5rem; }
.error-line { color: var(--red); font-size: 0.8rem; margin: 0.2rem 0; }
.detail-link { font-size: 0.8rem; margin-top: 0.5rem; display: inline-block; }

.sys-bar {
  display: flex;
  flex-wrap: wrap;
  gap: 1.5rem;
  padding: 0.6rem 1rem;
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: 6px;
  margin-bottom: 1.5rem;
  font-size: 0.82rem;
  color: var(--text-dim);
}

.channel-grid {
  display: flex;
  flex-wrap: wrap;
  gap: 0.5rem;
}
.channel-item {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  background: #21262d;
  padding: 0.3rem 0.8rem;
  border-radius: 4px;
  font-size: 0.82rem;
}
.channel-name { color: var(--text); }
.channel-count { color: var(--accent); font-weight: 600; }

.tbl { width: 100%; border-collapse: collapse; font-size: 0.85rem; }
.tbl th { color: var(--text-dim); font-weight: 600; text-align: left; padding: 0.4rem 0.6rem; border-bottom: 1px solid var(--border); }
.tbl td { padding: 0.4rem 0.6rem; border-bottom: 1px solid var(--border); }
</style>
