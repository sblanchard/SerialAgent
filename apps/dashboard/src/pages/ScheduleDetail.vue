<script setup lang="ts">
import { ref, onMounted, computed } from "vue";
import { useRouter } from "vue-router";
import { api, ApiError } from "@/api/client";
import type { Schedule, ScheduleDetailResponse } from "@/api/client";
import Card from "@/components/Card.vue";
import LoadingPanel from "@/components/LoadingPanel.vue";

const props = defineProps<{ id: string }>();
const router = useRouter();

const schedule = ref<Schedule | null>(null);
const nextOccurrences = ref<string[]>([]);
const loading = ref(true);
const error = ref("");

async function load() {
  loading.value = true;
  error.value = "";
  try {
    const res: ScheduleDetailResponse = await api.getSchedule(props.id);
    schedule.value = res.schedule;
    nextOccurrences.value = res.next_occurrences;
  } catch (e: unknown) {
    error.value = e instanceof ApiError ? e.friendly : String(e);
  } finally {
    loading.value = false;
  }
}

onMounted(load);

// ── Helpers ─────────────────────────────────────────────────────

const statusLabel = computed(() => {
  if (!schedule.value) return "";
  if (schedule.value.status === "error") return "Error";
  if (schedule.value.status === "paused") return "Paused";
  return "Active";
});

const statusClass = computed(() => {
  if (!schedule.value) return "";
  if (schedule.value.status === "error") return "status-error";
  if (schedule.value.enabled) return "status-enabled";
  return "status-paused";
});

function formatDate(iso?: string): string {
  if (!iso) return "-";
  return new Date(iso).toLocaleString();
}

function formatTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`;
  return String(n);
}

async function runNow() {
  if (!schedule.value) return;
  try {
    const res = await api.runScheduleNow(schedule.value.id);
    router.push(`/runs/${res.run_id}`);
  } catch (e: unknown) {
    error.value = e instanceof ApiError ? e.friendly : String(e);
  }
}

async function toggleEnabled() {
  if (!schedule.value) return;
  try {
    await api.updateSchedule(schedule.value.id, { enabled: !schedule.value.enabled });
    await load();
  } catch (e: unknown) {
    error.value = e instanceof ApiError ? e.friendly : String(e);
  }
}

function goBack() {
  router.push("/schedules");
}
</script>

<template>
  <div>
    <div class="header-row">
      <button class="back-btn" @click="goBack">&larr; Schedules</button>
      <h1 v-if="schedule" class="page-title">{{ schedule.name }}</h1>
    </div>

    <p v-if="error" class="error">{{ error }}</p>

    <LoadingPanel v-if="loading && !schedule" message="Loading schedule..." />

    <template v-if="schedule">
      <!-- Actions -->
      <div class="actions-bar">
        <button class="action-btn run-btn" @click="runNow">Run Now</button>
        <button
          class="action-btn toggle-btn"
          :class="schedule.enabled ? 'toggle-on' : 'toggle-off'"
          @click="toggleEnabled"
        >{{ schedule.enabled ? "Pause" : "Enable" }}</button>
        <span class="status-badge" :class="statusClass">{{ statusLabel }}</span>
      </div>

      <!-- Overview -->
      <Card title="Overview">
        <div class="detail-grid">
          <div class="detail-item">
            <span class="label">Cron</span>
            <span class="value mono">{{ schedule.cron }}</span>
          </div>
          <div class="detail-item">
            <span class="label">Timezone</span>
            <span class="value">{{ schedule.timezone }}</span>
          </div>
          <div class="detail-item">
            <span class="label">Missed Policy</span>
            <span class="value">{{ schedule.missed_policy }}</span>
          </div>
          <div class="detail-item">
            <span class="label">Max Concurrency</span>
            <span class="value">{{ schedule.max_concurrency }}</span>
          </div>
          <div class="detail-item">
            <span class="label">Timeout</span>
            <span class="value">{{ schedule.timeout_ms ? `${schedule.timeout_ms}ms` : "None" }}</span>
          </div>
          <div class="detail-item">
            <span class="label">Digest Mode</span>
            <span class="value">{{ schedule.digest_mode }}</span>
          </div>
          <div class="detail-item">
            <span class="label">Max Catch-up Runs</span>
            <span class="value">{{ schedule.max_catchup_runs }}</span>
          </div>
          <div class="detail-item">
            <span class="label">Created</span>
            <span class="value dim">{{ formatDate(schedule.created_at) }}</span>
          </div>
        </div>
      </Card>

      <!-- Next Occurrences -->
      <Card title="Next Occurrences">
        <ul v-if="nextOccurrences.length > 0" class="occurrences-list">
          <li v-for="(occ, i) in nextOccurrences" :key="i" class="mono">
            {{ formatDate(occ) }}
          </li>
        </ul>
        <p v-else class="dim">No upcoming occurrences</p>
      </Card>

      <!-- Usage -->
      <Card title="Usage">
        <div class="detail-grid">
          <div class="detail-item">
            <span class="label">Total Runs</span>
            <span class="value">{{ schedule.total_runs }}</span>
          </div>
          <div class="detail-item">
            <span class="label">Input Tokens</span>
            <span class="value">{{ formatTokens(schedule.total_input_tokens) }}</span>
          </div>
          <div class="detail-item">
            <span class="label">Output Tokens</span>
            <span class="value">{{ formatTokens(schedule.total_output_tokens) }}</span>
          </div>
        </div>
      </Card>

      <!-- Error History -->
      <Card v-if="schedule.consecutive_failures > 0 || schedule.last_error" title="Error History">
        <div class="detail-grid">
          <div class="detail-item">
            <span class="label">Consecutive Failures</span>
            <span class="value error-text">{{ schedule.consecutive_failures }}</span>
          </div>
          <div v-if="schedule.last_error_at" class="detail-item">
            <span class="label">Last Error At</span>
            <span class="value dim">{{ formatDate(schedule.last_error_at) }}</span>
          </div>
          <div v-if="schedule.cooldown_until" class="detail-item">
            <span class="label">Cooldown Until</span>
            <span class="value dim">{{ formatDate(schedule.cooldown_until) }}</span>
          </div>
        </div>
        <div v-if="schedule.last_error" class="error-box">
          <pre>{{ schedule.last_error }}</pre>
        </div>
      </Card>

      <!-- Source States -->
      <Card v-if="Object.keys(schedule.source_states).length > 0" title="Source States">
        <table class="tbl">
          <thead>
            <tr>
              <th>Source</th>
              <th>Last Fetched</th>
              <th>HTTP Status</th>
              <th>Error</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="(state, url) in schedule.source_states" :key="url">
              <td class="mono source-url">{{ url }}</td>
              <td class="dim">{{ formatDate(state.last_fetched_at) }}</td>
              <td>{{ state.last_http_status ?? "-" }}</td>
              <td class="error-text">{{ state.last_error ?? "-" }}</td>
            </tr>
          </tbody>
        </table>
      </Card>

      <!-- Fetch Config -->
      <Card title="Fetch Configuration">
        <div class="detail-grid">
          <div class="detail-item">
            <span class="label">Fetch Timeout</span>
            <span class="value">{{ schedule.fetch_config.timeout_ms }}ms</span>
          </div>
          <div class="detail-item">
            <span class="label">User-Agent</span>
            <span class="value mono">{{ schedule.fetch_config.user_agent }}</span>
          </div>
          <div class="detail-item">
            <span class="label">Max Body Size</span>
            <span class="value">{{ schedule.fetch_config.max_size_bytes === 0 ? "Unlimited" : `${schedule.fetch_config.max_size_bytes} bytes` }}</span>
          </div>
        </div>
      </Card>

      <!-- Prompt Template -->
      <Card title="Prompt Template">
        <pre class="prompt-box">{{ schedule.prompt_template }}</pre>
      </Card>

      <!-- Sources -->
      <Card v-if="schedule.sources.length > 0" title="Sources">
        <ul class="sources-list">
          <li v-for="(src, i) in schedule.sources" :key="i" class="mono">{{ src }}</li>
        </ul>
      </Card>
    </template>
  </div>
</template>

<style scoped>
.page-title { font-size: 1.5rem; color: var(--accent); margin: 0; }
.error { color: var(--red); margin-bottom: 1rem; }
.error-text { color: var(--red); }
.dim { color: var(--text-dim); font-size: 0.85rem; }
.mono { font-family: var(--mono); font-size: 0.82rem; }

.header-row {
  display: flex;
  align-items: center;
  gap: 1rem;
  margin-bottom: 1rem;
}

.back-btn {
  background: transparent;
  border: 1px solid var(--border);
  color: var(--text-dim);
  padding: 0.3rem 0.8rem;
  border-radius: 4px;
  font-size: 0.82rem;
  cursor: pointer;
}
.back-btn:hover { color: var(--text); border-color: var(--text-dim); }

.actions-bar {
  display: flex;
  align-items: center;
  gap: 0.6rem;
  margin-bottom: 1rem;
}

.action-btn {
  background: transparent;
  border: 1px solid var(--border);
  color: var(--text-dim);
  padding: 0.3rem 0.8rem;
  border-radius: 4px;
  font-size: 0.82rem;
  cursor: pointer;
}
.action-btn:hover { color: var(--text); border-color: var(--text-dim); }
.run-btn { color: var(--accent); border-color: var(--accent); }
.run-btn:hover { background: rgba(88, 166, 255, 0.1); }
.toggle-on { color: var(--text-dim); }
.toggle-off { color: var(--green); border-color: var(--green); }
.toggle-off:hover { background: rgba(63, 185, 80, 0.1); }

.status-badge {
  display: inline-block;
  padding: 0.15rem 0.5rem;
  border-radius: 3px;
  font-size: 0.78rem;
  font-weight: 500;
}
.status-enabled { background: rgba(63, 185, 80, 0.15); color: var(--green); }
.status-paused { background: rgba(139, 148, 158, 0.15); color: var(--text-dim); }
.status-error { background: rgba(248, 81, 73, 0.15); color: var(--red); }

.detail-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
  gap: 1rem;
}

.detail-item {
  display: flex;
  flex-direction: column;
  gap: 0.15rem;
}

.detail-item .label {
  font-size: 0.72rem;
  color: var(--text-dim);
  text-transform: uppercase;
  letter-spacing: 0.03em;
}

.detail-item .value {
  font-size: 0.88rem;
}

.occurrences-list {
  list-style: none;
  padding: 0;
  margin: 0;
}
.occurrences-list li {
  padding: 0.3rem 0;
  border-bottom: 1px solid var(--border);
}
.occurrences-list li:last-child { border-bottom: none; }

.error-box {
  margin-top: 0.8rem;
  background: rgba(248, 81, 73, 0.08);
  border: 1px solid rgba(248, 81, 73, 0.2);
  border-radius: 4px;
  padding: 0.6rem 0.8rem;
}
.error-box pre {
  margin: 0;
  white-space: pre-wrap;
  word-break: break-word;
  font-family: var(--mono);
  font-size: 0.82rem;
  color: var(--red);
}

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

.source-url {
  max-width: 300px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.prompt-box {
  background: var(--bg);
  border: 1px solid var(--border);
  border-radius: 4px;
  padding: 0.8rem;
  font-family: var(--mono);
  font-size: 0.82rem;
  white-space: pre-wrap;
  word-break: break-word;
  margin: 0;
}

.sources-list {
  list-style: none;
  padding: 0;
  margin: 0;
}
.sources-list li {
  padding: 0.3rem 0;
  border-bottom: 1px solid var(--border);
}
.sources-list li:last-child { border-bottom: none; }
</style>
