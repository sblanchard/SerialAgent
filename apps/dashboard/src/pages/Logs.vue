<script setup lang="ts">
import { ref, onMounted, onUnmounted, computed } from "vue";
import { api, ApiError } from "@/api/client";
import type { RunListItem } from "@/api/client";
import Card from "@/components/Card.vue";
import RunStatusBadge from "@/components/RunStatusBadge.vue";
import LoadingPanel from "@/components/LoadingPanel.vue";

const runs = ref<RunListItem[]>([]);
const loading = ref(false);
const error = ref("");
const filterText = ref("");
const filterStatus = ref<string>("");

let pollTimer: ReturnType<typeof setInterval> | null = null;

async function load() {
  loading.value = true;
  error.value = "";
  try {
    const res = await api.getRuns({
      status: (filterStatus.value as any) || undefined,
      limit: 100,
    });
    runs.value = res.runs;
  } catch (e: unknown) {
    error.value = e instanceof ApiError ? e.friendly : String(e);
  } finally {
    loading.value = false;
  }
}

const filteredRuns = computed(() => {
  if (!filterText.value) return runs.value;
  const q = filterText.value.toLowerCase();
  return runs.value.filter(
    (r) =>
      r.session_key.toLowerCase().includes(q) ||
      (r.input_preview || "").toLowerCase().includes(q) ||
      (r.output_preview || "").toLowerCase().includes(q) ||
      (r.agent_id || "").toLowerCase().includes(q) ||
      (r.error || "").toLowerCase().includes(q) ||
      r.run_id.toLowerCase().includes(q)
  );
});

function formatDuration(ms?: number): string {
  if (ms == null) return "-";
  if (ms < 1000) return `${ms}ms`;
  return `${(ms / 1000).toFixed(1)}s`;
}

function formatTokens(n: number): string {
  if (n < 1000) return String(n);
  return `${(n / 1000).toFixed(1)}k`;
}

function timeAgo(iso: string): string {
  const secs = Math.floor((Date.now() - new Date(iso).getTime()) / 1000);
  if (secs < 60) return `${secs}s ago`;
  if (secs < 3600) return `${Math.floor(secs / 60)}m ago`;
  if (secs < 86400) return `${Math.floor(secs / 3600)}h ago`;
  return `${Math.floor(secs / 86400)}d ago`;
}

function nodeBreakdown(run: RunListItem): string {
  return `${run.node_count} nodes, ${run.loop_count} loops`;
}

onMounted(() => {
  load();
  pollTimer = setInterval(load, 5000);
});

onUnmounted(() => {
  if (pollTimer) clearInterval(pollTimer);
});
</script>

<template>
  <div>
    <h1 class="page-title">Logs & Debug</h1>

    <div class="filter-bar">
      <input
        v-model="filterText"
        class="filter-input"
        placeholder="Search runs by session, input, output, agent, error..."
      />
      <select v-model="filterStatus" class="filter-select" @change="load">
        <option value="">All statuses</option>
        <option value="running">Running</option>
        <option value="completed">Completed</option>
        <option value="failed">Failed</option>
        <option value="stopped">Stopped</option>
      </select>
      <button class="refresh-btn" @click="load" :disabled="loading">Refresh</button>
      <span class="total dim">{{ filteredRuns.length }} entries</span>
    </div>

    <p v-if="error" class="error">{{ error }}</p>

    <Card v-if="loading && runs.length === 0" title="Loading">
      <LoadingPanel message="Loading logs..." />
    </Card>

    <Card v-if="filteredRuns.length > 0" title="Run Logs">
      <div class="log-list">
        <div
          v-for="run in filteredRuns"
          :key="run.run_id"
          class="log-entry"
          :class="{ 'is-error': run.status === 'failed' }"
        >
          <div class="log-header">
            <RunStatusBadge :status="run.status" size="sm" />
            <span class="log-id mono">{{ run.run_id.substring(0, 8) }}</span>
            <span class="log-session dim">{{ run.session_key }}</span>
            <span class="log-agent" v-if="run.agent_id">agent:{{ run.agent_id }}</span>
            <span class="log-time dim">{{ timeAgo(run.started_at) }}</span>
          </div>
          <div class="log-detail">
            <div v-if="run.input_preview" class="log-field">
              <span class="field-label">IN</span>
              <span class="field-val">{{ run.input_preview }}</span>
            </div>
            <div v-if="run.output_preview" class="log-field">
              <span class="field-label">OUT</span>
              <span class="field-val">{{ run.output_preview }}</span>
            </div>
            <div v-if="run.error" class="log-field error-field">
              <span class="field-label">ERR</span>
              <span class="field-val">{{ run.error }}</span>
            </div>
          </div>
          <div class="log-meta mono">
            {{ formatDuration(run.duration_ms) }} |
            {{ formatTokens(run.total_tokens) }} tokens |
            {{ nodeBreakdown(run) }}
            <router-link :to="`/runs/${run.run_id}`" class="detail-link">View Run</router-link>
          </div>
        </div>
      </div>
    </Card>

    <Card v-if="!loading && filteredRuns.length === 0 && !error" title="No Logs">
      <div class="empty dim">No run logs found. Runs are created when chat or scheduled tasks execute.</div>
    </Card>
  </div>
</template>

<style scoped>
.page-title { font-size: 1.5rem; margin-bottom: 1.5rem; color: var(--accent); }
.error { color: var(--red); margin-bottom: 1rem; }
.dim { color: var(--text-dim); font-size: 0.85rem; }
.mono { font-family: var(--mono); font-size: 0.82rem; }

.filter-bar {
  display: flex;
  align-items: center;
  gap: 0.6rem;
  margin-bottom: 1rem;
}
.filter-input {
  flex: 1;
  background: var(--bg);
  border: 1px solid var(--border);
  color: var(--text);
  padding: 0.35rem 0.6rem;
  border-radius: 4px;
  font-size: 0.82rem;
}
.filter-input:focus { outline: none; border-color: var(--accent); }
.filter-select {
  background: var(--bg);
  border: 1px solid var(--border);
  color: var(--text);
  padding: 0.35rem 0.6rem;
  border-radius: 4px;
  font-size: 0.82rem;
}
.refresh-btn {
  background: transparent;
  border: 1px solid var(--border);
  color: var(--text-dim);
  padding: 0.35rem 0.8rem;
  border-radius: 4px;
  font-size: 0.82rem;
  cursor: pointer;
}
.refresh-btn:hover:not(:disabled) { color: var(--text); border-color: var(--text-dim); }
.refresh-btn:disabled { opacity: 0.5; cursor: not-allowed; }
.total { margin-left: auto; }

.log-list {
  display: flex;
  flex-direction: column;
}
.log-entry {
  padding: 0.5rem 0.6rem;
  border-bottom: 1px solid var(--border);
}
.log-entry.is-error { border-left: 2px solid var(--red); }
.log-header {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  margin-bottom: 0.2rem;
}
.log-id { color: var(--text); }
.log-session { font-size: 0.78rem; max-width: 200px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.log-agent { font-size: 0.75rem; color: var(--accent); background: rgba(88,166,255,0.1); padding: 0.1rem 0.4rem; border-radius: 3px; }
.log-time { margin-left: auto; font-size: 0.75rem; }

.log-detail {
  margin: 0.2rem 0;
}
.log-field {
  display: flex;
  gap: 0.4rem;
  font-size: 0.8rem;
  margin-bottom: 0.1rem;
}
.field-label {
  font-weight: 700;
  font-size: 0.68rem;
  color: var(--text-dim);
  min-width: 2rem;
  font-family: var(--mono);
}
.field-val {
  color: var(--text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  max-width: 600px;
}
.error-field .field-val { color: var(--red); }

.log-meta {
  font-size: 0.75rem;
  color: var(--text-dim);
  display: flex;
  gap: 0.3rem;
  align-items: center;
}
.detail-link {
  color: var(--accent);
  text-decoration: none;
  margin-left: auto;
  font-size: 0.75rem;
}
.detail-link:hover { text-decoration: underline; }

.empty { text-align: center; padding: 2rem; }
</style>
