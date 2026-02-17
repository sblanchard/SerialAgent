<script setup lang="ts">
import { ref, onMounted, computed, watch } from "vue";
import { useRouter } from "vue-router";
import { api, ApiError } from "@/api/client";
import type { RunListItem, RunStatus as RunStatusType } from "@/api/client";
import Card from "@/components/Card.vue";
import RunStatusBadge from "@/components/RunStatusBadge.vue";
import EmptyState from "@/components/EmptyState.vue";
import LoadingPanel from "@/components/LoadingPanel.vue";

const router = useRouter();

const runs = ref<RunListItem[]>([]);
const total = ref(0);
const loading = ref(false);
const error = ref("");

// Filters
const statusFilter = ref<RunStatusType | "">("");
const page = ref(0);
const pageSize = 25;

// Polling
let pollTimer: ReturnType<typeof setInterval> | null = null;

async function load() {
  loading.value = true;
  error.value = "";
  try {
    const res = await api.getRuns({
      status: statusFilter.value || undefined,
      limit: pageSize,
      offset: page.value * pageSize,
    });
    runs.value = res.runs;
    total.value = res.total;
  } catch (e: unknown) {
    error.value = e instanceof ApiError ? e.friendly : String(e);
  } finally {
    loading.value = false;
  }
}

function startPolling() {
  stopPolling();
  pollTimer = setInterval(load, 5000);
}

function stopPolling() {
  if (pollTimer) {
    clearInterval(pollTimer);
    pollTimer = null;
  }
}

watch(statusFilter, () => {
  page.value = 0;
  load();
});

watch(page, load);

onMounted(() => {
  load();
  startPolling();
});

// Cleanup on unmount
import { onUnmounted } from "vue";
onUnmounted(stopPolling);

const hasMore = computed(() => (page.value + 1) * pageSize < total.value);

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

function goToRun(runId: string) {
  router.push(`/runs/${runId}`);
}
</script>

<template>
  <div>
    <h1 class="page-title">Runs</h1>

    <!-- Filters -->
    <div class="filter-bar">
      <select v-model="statusFilter" class="filter-select">
        <option value="">All statuses</option>
        <option value="running">Running</option>
        <option value="completed">Completed</option>
        <option value="failed">Failed</option>
        <option value="stopped">Stopped</option>
        <option value="queued">Queued</option>
      </select>
      <button class="refresh-btn" @click="load" :disabled="loading">Refresh</button>
      <span class="total dim">{{ total }} total</span>
    </div>

    <p v-if="error" class="error">{{ error }}</p>

    <Card v-if="loading && runs.length === 0" title="Loading">
      <LoadingPanel message="Loading runs..." />
    </Card>

    <Card v-if="!loading && runs.length === 0 && !error" title="No Runs">
      <EmptyState
        icon=">"
        title="No runs found"
        description="Runs are created when chat or agent turns execute. Send a message via /v1/chat to create a run."
      />
    </Card>

    <Card v-if="runs.length > 0" title="Recent Runs">
      <table class="tbl">
        <thead>
          <tr>
            <th>Status</th>
            <th>Input</th>
            <th>Output</th>
            <th>Duration</th>
            <th>Tokens</th>
            <th>Nodes</th>
            <th>Started</th>
          </tr>
        </thead>
        <tbody>
          <tr
            v-for="run in runs"
            :key="run.run_id"
            class="clickable"
            @click="goToRun(run.run_id)"
          >
            <td><RunStatusBadge :status="run.status" size="sm" /></td>
            <td class="preview">{{ run.input_preview || "-" }}</td>
            <td class="preview">
              <span v-if="run.error" class="error-text">{{ run.error }}</span>
              <span v-else>{{ run.output_preview || "-" }}</span>
            </td>
            <td class="mono">{{ formatDuration(run.duration_ms) }}</td>
            <td class="mono">{{ formatTokens(run.total_tokens) }}</td>
            <td class="mono">{{ run.node_count }}</td>
            <td class="dim">{{ timeAgo(run.started_at) }}</td>
          </tr>
        </tbody>
      </table>

      <!-- Pagination -->
      <div class="pagination">
        <button class="secondary" @click="page--" :disabled="page === 0">Previous</button>
        <span class="dim">
          {{ page * pageSize + 1 }}-{{ Math.min((page + 1) * pageSize, total) }} of {{ total }}
        </span>
        <button class="secondary" @click="page++" :disabled="!hasMore">Next</button>
      </div>
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

.tbl { width: 100%; border-collapse: collapse; font-size: 0.85rem; }
.tbl th {
  color: var(--text-dim);
  font-weight: 600;
  text-align: left;
  padding: 0.5rem 0.6rem;
  border-bottom: 1px solid var(--border);
  font-size: 0.78rem;
  text-transform: uppercase;
  letter-spacing: 0.03em;
}
.tbl td { padding: 0.5rem 0.6rem; border-bottom: 1px solid var(--border); }

.clickable { cursor: pointer; }
.clickable:hover { background: rgba(88, 166, 255, 0.03); }

.preview {
  max-width: 250px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 0.82rem;
}

.error-text { color: var(--red); font-size: 0.82rem; }

.pagination {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 1rem;
  margin-top: 0.8rem;
  padding-top: 0.5rem;
}

button.secondary {
  background: transparent;
  border: 1px solid var(--border);
  color: var(--text-dim);
  padding: 0.3rem 0.8rem;
  border-radius: 4px;
  font-size: 0.78rem;
  cursor: pointer;
}
button.secondary:disabled { opacity: 0.5; cursor: not-allowed; }
button.secondary:hover:not(:disabled) { color: var(--text); border-color: var(--text-dim); }
</style>
