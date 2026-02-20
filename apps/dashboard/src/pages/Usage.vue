<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted } from "vue";
import { useRouter } from "vue-router";
import { api, ApiError } from "@/api/client";
import type { RunListItem, RunListResponse, QuotaStatus } from "@/api/client";
import Card from "@/components/Card.vue";
import LoadingPanel from "@/components/LoadingPanel.vue";

const router = useRouter();

const runs = ref<RunListItem[]>([]);
const quotas = ref<QuotaStatus[]>([]);
const loading = ref(false);
const error = ref("");

let pollTimer: ReturnType<typeof setInterval> | null = null;

async function load() {
  loading.value = true;
  error.value = "";
  try {
    const [runsRes, quotasRes] = await Promise.all([
      api.getRuns({ limit: 200 }),
      api.getQuotas().catch(() => ({ quotas: [] as QuotaStatus[] })),
    ]);
    runs.value = runsRes.runs;
    quotas.value = quotasRes.quotas;
  } catch (e: unknown) {
    error.value = e instanceof ApiError ? e.friendly : String(e);
  } finally {
    loading.value = false;
  }
}

function startPolling() {
  stopPolling();
  pollTimer = setInterval(load, 30000);
}

function stopPolling() {
  if (pollTimer) {
    clearInterval(pollTimer);
    pollTimer = null;
  }
}

onMounted(() => {
  load();
  startPolling();
});

onUnmounted(stopPolling);

// ── Helpers ───────────────────────────────────────────────────────

function formatTokens(n: number): string {
  if (n < 1000) return String(n);
  return `${(n / 1000).toFixed(1)}k`;
}

function formatDuration(ms?: number): string {
  if (ms == null) return "-";
  if (ms < 1000) return `${ms}ms`;
  return `${(ms / 1000).toFixed(1)}s`;
}

function timeAgo(iso: string): string {
  const secs = Math.floor((Date.now() - new Date(iso).getTime()) / 1000);
  if (secs < 60) return `${secs}s ago`;
  if (secs < 3600) return `${Math.floor(secs / 60)}m ago`;
  if (secs < 86400) return `${Math.floor(secs / 3600)}h ago`;
  return `${Math.floor(secs / 86400)}d ago`;
}

// ── Summary stats ─────────────────────────────────────────────────

const totalRuns = computed(() => runs.value.length);

const totalInputTokens = computed(() =>
  runs.value.reduce((sum, r) => sum + r.input_tokens, 0)
);

const totalOutputTokens = computed(() =>
  runs.value.reduce((sum, r) => sum + r.output_tokens, 0)
);

const totalTokens = computed(() =>
  runs.value.reduce((sum, r) => sum + r.total_tokens, 0)
);

const avgTokensPerRun = computed(() =>
  totalRuns.value > 0 ? Math.round(totalTokens.value / totalRuns.value) : 0
);

// ── Breakdown by status ───────────────────────────────────────────

type StatusBucket = {
  status: string;
  count: number;
  input: number;
  output: number;
  total: number;
};

const byStatus = computed<StatusBucket[]>(() => {
  const map = new Map<string, StatusBucket>();
  for (const r of runs.value) {
    let bucket = map.get(r.status);
    if (!bucket) {
      bucket = { status: r.status, count: 0, input: 0, output: 0, total: 0 };
      map.set(r.status, bucket);
    }
    bucket.count++;
    bucket.input += r.input_tokens;
    bucket.output += r.output_tokens;
    bucket.total += r.total_tokens;
  }
  return [...map.values()].sort((a, b) => b.total - a.total);
});

// ── Breakdown by model ────────────────────────────────────────────

type ModelBucket = {
  model: string;
  count: number;
  input: number;
  output: number;
  total: number;
};

const byModel = computed<ModelBucket[]>(() => {
  const map = new Map<string, ModelBucket>();
  for (const r of runs.value) {
    const key = r.model || "(default)";
    let bucket = map.get(key);
    if (!bucket) {
      bucket = { model: key, count: 0, input: 0, output: 0, total: 0 };
      map.set(key, bucket);
    }
    bucket.count++;
    bucket.input += r.input_tokens;
    bucket.output += r.output_tokens;
    bucket.total += r.total_tokens;
  }
  return [...map.values()].sort((a, b) => b.total - a.total);
});

// ── Breakdown by agent ────────────────────────────────────────────

type AgentBucket = {
  agent: string;
  count: number;
  input: number;
  output: number;
  total: number;
};

const byAgent = computed<AgentBucket[]>(() => {
  const map = new Map<string, AgentBucket>();
  for (const r of runs.value) {
    const key = r.agent_id || "(default)";
    let bucket = map.get(key);
    if (!bucket) {
      bucket = { agent: key, count: 0, input: 0, output: 0, total: 0 };
      map.set(key, bucket);
    }
    bucket.count++;
    bucket.input += r.input_tokens;
    bucket.output += r.output_tokens;
    bucket.total += r.total_tokens;
  }
  return [...map.values()].sort((a, b) => b.total - a.total);
});

// ── Recent runs (top 20 for the table) ────────────────────────────

const recentRuns = computed(() => runs.value.slice(0, 20));

function goToRun(runId: string) {
  router.push(`/runs/${runId}`);
}

function statusColor(status: string): string {
  switch (status) {
    case "completed": return "var(--green)";
    case "failed": return "var(--red)";
    case "running": return "var(--accent)";
    case "stopped": return "var(--text-dim)";
    default: return "var(--text-dim)";
  }
}

// ── Quota helpers ─────────────────────────────────────────────────

const quotasWithLimits = computed(() =>
  quotas.value.filter(q => q.tokens_limit != null || q.cost_limit_usd != null)
);

function quotaPercent(used: number, limit: number | null): number {
  if (limit == null || limit === 0) return 0;
  return Math.min(100, (used / limit) * 100);
}

function quotaBarColor(pct: number): string {
  if (pct >= 90) return "var(--red)";
  if (pct >= 75) return "var(--yellow, #e6a700)";
  return "var(--green)";
}

function formatCost(usd: number): string {
  return `$${usd.toFixed(4)}`;
}
</script>

<template>
  <div>
    <h1 class="page-title">Token Usage</h1>

    <p v-if="error" class="error">{{ error }}</p>

    <Card v-if="loading && runs.length === 0" title="Loading">
      <LoadingPanel message="Loading usage data..." />
    </Card>

    <template v-if="runs.length > 0 || (!loading && !error)">
      <!-- Summary stats -->
      <div class="stats-grid">
        <div class="stat-card">
          <span class="stat-num">{{ totalRuns }}</span>
          <span class="stat-label">Total Runs</span>
        </div>
        <div class="stat-card">
          <span class="stat-num">{{ formatTokens(totalInputTokens) }}</span>
          <span class="stat-label">Input Tokens</span>
        </div>
        <div class="stat-card">
          <span class="stat-num">{{ formatTokens(totalOutputTokens) }}</span>
          <span class="stat-label">Output Tokens</span>
        </div>
        <div class="stat-card">
          <span class="stat-num">{{ formatTokens(totalTokens) }}</span>
          <span class="stat-label">Total Tokens</span>
        </div>
        <div class="stat-card">
          <span class="stat-num">{{ formatTokens(avgTokensPerRun) }}</span>
          <span class="stat-label">Avg / Run</span>
        </div>
      </div>

      <!-- Daily Quotas -->
      <Card v-if="quotasWithLimits.length > 0" title="Daily Quotas">
        <div class="quota-list">
          <div v-for="q in quotasWithLimits" :key="q.agent_id" class="quota-row">
            <div class="quota-agent mono">{{ q.agent_id }}</div>
            <div v-if="q.tokens_limit != null" class="quota-bar-group">
              <div class="quota-bar-label">
                <span>Tokens</span>
                <span class="mono">{{ formatTokens(q.tokens_used) }} / {{ formatTokens(q.tokens_limit) }}</span>
              </div>
              <div class="quota-bar-track">
                <div
                  class="quota-bar-fill"
                  :style="{
                    width: quotaPercent(q.tokens_used, q.tokens_limit) + '%',
                    background: quotaBarColor(quotaPercent(q.tokens_used, q.tokens_limit)),
                  }"
                ></div>
              </div>
            </div>
            <div v-if="q.cost_limit_usd != null" class="quota-bar-group">
              <div class="quota-bar-label">
                <span>Cost</span>
                <span class="mono">{{ formatCost(q.cost_used_usd) }} / {{ formatCost(q.cost_limit_usd) }}</span>
              </div>
              <div class="quota-bar-track">
                <div
                  class="quota-bar-fill"
                  :style="{
                    width: quotaPercent(q.cost_used_usd, q.cost_limit_usd) + '%',
                    background: quotaBarColor(quotaPercent(q.cost_used_usd, q.cost_limit_usd)),
                  }"
                ></div>
              </div>
            </div>
          </div>
        </div>
      </Card>

      <!-- Breakdown by status -->
      <Card title="Tokens by Status">
        <div v-if="byStatus.length === 0" class="empty-msg dim">No data</div>
        <table v-else class="tbl">
          <thead>
            <tr>
              <th>Status</th>
              <th>Runs</th>
              <th>Input</th>
              <th>Output</th>
              <th>Total</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="b in byStatus" :key="b.status">
              <td>
                <span class="status-badge" :style="{ color: statusColor(b.status) }">{{ b.status }}</span>
              </td>
              <td class="mono">{{ b.count }}</td>
              <td class="mono">{{ formatTokens(b.input) }}</td>
              <td class="mono">{{ formatTokens(b.output) }}</td>
              <td class="mono accent">{{ formatTokens(b.total) }}</td>
            </tr>
          </tbody>
        </table>
      </Card>

      <!-- Breakdown by model -->
      <Card title="Tokens by Model">
        <div v-if="byModel.length === 0" class="empty-msg dim">No data</div>
        <table v-else class="tbl">
          <thead>
            <tr>
              <th>Model</th>
              <th>Runs</th>
              <th>Input</th>
              <th>Output</th>
              <th>Total</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="b in byModel" :key="b.model">
              <td class="mono">{{ b.model }}</td>
              <td class="mono">{{ b.count }}</td>
              <td class="mono">{{ formatTokens(b.input) }}</td>
              <td class="mono">{{ formatTokens(b.output) }}</td>
              <td class="mono accent">{{ formatTokens(b.total) }}</td>
            </tr>
          </tbody>
        </table>
      </Card>

      <!-- Breakdown by agent -->
      <Card title="Tokens by Agent">
        <div v-if="byAgent.length === 0" class="empty-msg dim">No data</div>
        <table v-else class="tbl">
          <thead>
            <tr>
              <th>Agent</th>
              <th>Runs</th>
              <th>Input</th>
              <th>Output</th>
              <th>Total</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="b in byAgent" :key="b.agent">
              <td class="mono">{{ b.agent }}</td>
              <td class="mono">{{ b.count }}</td>
              <td class="mono">{{ formatTokens(b.input) }}</td>
              <td class="mono">{{ formatTokens(b.output) }}</td>
              <td class="mono accent">{{ formatTokens(b.total) }}</td>
            </tr>
          </tbody>
        </table>
      </Card>

      <!-- Recent runs table -->
      <Card title="Recent Runs">
        <div v-if="recentRuns.length === 0" class="empty-msg dim">No runs recorded</div>
        <table v-else class="tbl">
          <thead>
            <tr>
              <th>Status</th>
              <th>Agent</th>
              <th>Model</th>
              <th>Duration</th>
              <th>Input</th>
              <th>Output</th>
              <th>Total</th>
              <th>Started</th>
            </tr>
          </thead>
          <tbody>
            <tr
              v-for="run in recentRuns"
              :key="run.run_id"
              class="clickable"
              @click="goToRun(run.run_id)"
            >
              <td>
                <span class="status-badge" :style="{ color: statusColor(run.status) }">{{ run.status }}</span>
              </td>
              <td class="mono">{{ run.agent_id || "default" }}</td>
              <td class="mono">{{ run.model || "default" }}</td>
              <td class="mono">{{ formatDuration(run.duration_ms) }}</td>
              <td class="mono">{{ formatTokens(run.input_tokens) }}</td>
              <td class="mono">{{ formatTokens(run.output_tokens) }}</td>
              <td class="mono accent">{{ formatTokens(run.total_tokens) }}</td>
              <td class="dim">{{ timeAgo(run.started_at) }}</td>
            </tr>
          </tbody>
        </table>
      </Card>
    </template>
  </div>
</template>

<style scoped>
.page-title {
  font-size: 1.5rem;
  margin-bottom: 1.5rem;
  color: var(--accent);
}

.error {
  color: var(--red);
  margin-bottom: 1rem;
}

.dim {
  color: var(--text-dim);
  font-size: 0.85rem;
}

.mono {
  font-family: var(--mono);
  font-size: 0.82rem;
}

.accent {
  color: var(--accent);
  font-weight: 600;
}

.empty-msg {
  padding: 1rem 0;
  text-align: center;
}

/* ── Summary stats grid ─────────────────────────────────────────── */

.stats-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(150px, 1fr));
  gap: 1rem;
  margin-bottom: 1.5rem;
}

.stat-card {
  display: flex;
  flex-direction: column;
  align-items: center;
  padding: 1rem 0.8rem;
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: 6px;
}

.stat-num {
  font-size: 1.6rem;
  font-weight: 700;
  color: var(--text);
}

.stat-label {
  font-size: 0.78rem;
  color: var(--text-dim);
  margin-top: 0.2rem;
  text-transform: uppercase;
  letter-spacing: 0.04em;
}

/* ── Tables ─────────────────────────────────────────────────────── */

.tbl {
  width: 100%;
  border-collapse: collapse;
  font-size: 0.85rem;
}

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

.tbl td {
  padding: 0.5rem 0.6rem;
  border-bottom: 1px solid var(--border);
}

.clickable {
  cursor: pointer;
}

.clickable:hover {
  background: rgba(88, 166, 255, 0.03);
}

/* ── Quota bars ────────────────────────────────────────────────── */

.quota-list {
  display: flex;
  flex-direction: column;
  gap: 1.2rem;
}

.quota-row {
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
}

.quota-agent {
  font-weight: 600;
  font-size: 0.85rem;
  color: var(--text);
}

.quota-bar-group {
  display: flex;
  flex-direction: column;
  gap: 0.2rem;
}

.quota-bar-label {
  display: flex;
  justify-content: space-between;
  font-size: 0.78rem;
  color: var(--text-dim);
}

.quota-bar-track {
  height: 8px;
  background: var(--border);
  border-radius: 4px;
  overflow: hidden;
}

.quota-bar-fill {
  height: 100%;
  border-radius: 4px;
  transition: width 0.3s ease;
}

/* ── Status badge ───────────────────────────────────────────────── */

.status-badge {
  font-size: 0.78rem;
  font-weight: 600;
  text-transform: capitalize;
}
</style>
