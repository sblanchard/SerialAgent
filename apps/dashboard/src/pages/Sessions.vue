<script setup lang="ts">
import { ref, computed, onMounted } from "vue";
import { api } from "@/api/client";
import type { SessionEntry } from "@/api/client";
import Card from "@/components/Card.vue";
import StatusDot from "@/components/StatusDot.vue";

const sessions = ref<SessionEntry[]>([]);
const error = ref("");
const search = ref("");
const copied = ref("");

const filtered = computed(() => {
  const q = search.value.toLowerCase().trim();
  if (!q) return sessions.value;
  return sessions.value.filter(
    (s) =>
      s.session_key.toLowerCase().includes(q) ||
      (s.origin?.channel ?? "").toLowerCase().includes(q) ||
      (s.origin?.peer ?? "").toLowerCase().includes(q) ||
      (s.model ?? "").toLowerCase().includes(q)
  );
});

async function load() {
  try {
    const res = await api.sessions();
    sessions.value = res.sessions;
  } catch (e: any) {
    error.value = e.message;
  }
}

function copyKey(key: string) {
  navigator.clipboard.writeText(key).then(() => {
    copied.value = key;
    setTimeout(() => { copied.value = ""; }, 1500);
  });
}

async function stopSession(key: string) {
  try {
    await api.stopSession(key);
    await load(); // refresh list
  } catch (e: any) {
    error.value = e.message;
  }
}

async function resetSession(key: string) {
  try {
    await api.resetSession(key);
    await load();
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

    <div class="toolbar">
      <input
        v-model="search"
        class="search"
        placeholder="Filter by key, channel, peer, model..."
      />
      <span class="dim">{{ filtered.length }} of {{ sessions.length }}</span>
    </div>

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
            <th></th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="s in filtered" :key="s.session_key">
            <td>
              <StatusDot :status="s.running ? 'ok' : 'off'" />
            </td>
            <td>
              <router-link :to="{ name: 'session-detail', params: { key: s.session_key } }">
                <code>{{ s.session_key }}</code>
              </router-link>
              <button
                class="copy-btn"
                :class="{ copied: copied === s.session_key }"
                @click.stop="copyKey(s.session_key)"
                :title="copied === s.session_key ? 'Copied!' : 'Copy session key'"
              >
                {{ copied === s.session_key ? "ok" : "cp" }}
              </button>
            </td>
            <td>{{ s.origin?.channel || "-" }}</td>
            <td>{{ s.origin?.peer || "-" }}</td>
            <td class="dim"><code>{{ s.model || "-" }}</code></td>
            <td class="dim">{{ s.total_tokens?.toLocaleString() ?? 0 }}</td>
            <td class="dim">{{ timeSince(s.updated_at) }}</td>
            <td class="actions">
              <button
                v-if="s.running"
                class="action-btn stop"
                @click.stop="stopSession(s.session_key)"
                title="Cancel running turn"
              >stop</button>
              <button
                class="action-btn reset"
                @click.stop="resetSession(s.session_key)"
                title="Reset session"
              >reset</button>
            </td>
          </tr>
          <tr v-if="filtered.length === 0">
            <td colspan="8" class="dim" style="text-align:center;padding:2rem">
              {{ sessions.length === 0 ? "No sessions" : "No matches" }}
            </td>
          </tr>
        </tbody>
      </table>
    </Card>
  </div>
</template>

<style scoped>
.page-title { font-size: 1.5rem; margin-bottom: 1.5rem; color: var(--accent); }
.error { color: var(--red); margin-bottom: 1rem; }
.dim { color: var(--text-dim); font-size: 0.85rem; }

.toolbar {
  display: flex;
  align-items: center;
  gap: 1rem;
  margin-bottom: 1rem;
}
.search {
  flex: 1;
  background: var(--bg);
  border: 1px solid var(--border);
  color: var(--text);
  padding: 0.45rem 0.8rem;
  border-radius: 4px;
  font-size: 0.88rem;
  font-family: var(--mono);
}
.search::placeholder { color: var(--text-dim); }

.tbl { width: 100%; border-collapse: collapse; font-size: 0.85rem; }
.tbl th { color: var(--text-dim); font-weight: 600; text-align: left; padding: 0.5rem 0.6rem; border-bottom: 1px solid var(--border); }
.tbl td { padding: 0.5rem 0.6rem; border-bottom: 1px solid var(--border); }

.copy-btn {
  display: inline-block;
  margin-left: 0.4rem;
  background: #21262d;
  border: 1px solid var(--border);
  color: var(--text-dim);
  padding: 0.1rem 0.4rem;
  border-radius: 3px;
  cursor: pointer;
  font-family: var(--mono);
  font-size: 0.7rem;
}
.copy-btn:hover { color: var(--text); border-color: var(--text-dim); }
.copy-btn.copied { color: var(--green); border-color: var(--green); }

.actions { white-space: nowrap; }
.action-btn {
  background: transparent;
  border: 1px solid var(--border);
  color: var(--text-dim);
  padding: 0.15rem 0.5rem;
  border-radius: 3px;
  cursor: pointer;
  font-family: var(--mono);
  font-size: 0.72rem;
  margin-left: 0.3rem;
}
.action-btn:hover { color: var(--text); }
.action-btn.stop { border-color: var(--red); color: var(--red); }
.action-btn.stop:hover { background: rgba(248, 81, 73, 0.1); }
.action-btn.reset { border-color: var(--yellow); color: var(--yellow); }
.action-btn.reset:hover { background: rgba(210, 153, 34, 0.1); }
</style>
