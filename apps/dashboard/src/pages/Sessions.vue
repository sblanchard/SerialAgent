<script setup lang="ts">
import { ref, onMounted } from "vue";
import { api } from "@/api/client";
import type { SessionEntry } from "@/api/client";
import Card from "@/components/Card.vue";
import StatusDot from "@/components/StatusDot.vue";

const sessions = ref<SessionEntry[]>([]);
const error = ref("");

async function load() {
  try {
    const res = await api.sessions();
    sessions.value = res.sessions;
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
    <h1 class="page-title">Sessions</h1>
    <p v-if="error" class="error">{{ error }}</p>

    <Card>
      <table class="tbl">
        <thead>
          <tr>
            <th></th>
            <th>Session Key</th>
            <th>Channel</th>
            <th>Peer</th>
            <th>Model</th>
            <th>Tokens</th>
            <th>Last Touched</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="s in sessions" :key="s.session_key">
            <td>
              <StatusDot :status="s.running ? 'ok' : 'off'" />
            </td>
            <td>
              <router-link :to="{ name: 'session-detail', params: { key: s.session_key } }">
                <code>{{ s.session_key }}</code>
              </router-link>
            </td>
            <td>{{ s.origin?.channel || "-" }}</td>
            <td>{{ s.origin?.peer || "-" }}</td>
            <td class="dim"><code>{{ s.model || "-" }}</code></td>
            <td class="dim">{{ s.total_tokens?.toLocaleString() ?? 0 }}</td>
            <td class="dim">{{ timeSince(s.updated_at) }}</td>
          </tr>
          <tr v-if="sessions.length === 0">
            <td colspan="7" class="dim" style="text-align:center;padding:2rem">No sessions</td>
          </tr>
        </tbody>
      </table>
    </Card>
  </div>
</template>

<style scoped>
.page-title { font-size: 1.5rem; margin-bottom: 1.5rem; color: var(--accent); }
.error { color: var(--red); margin-bottom: 1rem; }
.dim { color: var(--text-dim); }
.tbl { width: 100%; border-collapse: collapse; font-size: 0.85rem; }
.tbl th { color: var(--text-dim); font-weight: 600; text-align: left; padding: 0.5rem 0.6rem; border-bottom: 1px solid var(--border); }
.tbl td { padding: 0.5rem 0.6rem; border-bottom: 1px solid var(--border); }
</style>
