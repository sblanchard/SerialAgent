<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted } from "vue";
import { useRouter } from "vue-router";
import { api, ApiError } from "@/api/client";
import type { Schedule, CreateScheduleRequest } from "@/api/client";
import Card from "@/components/Card.vue";
import EmptyState from "@/components/EmptyState.vue";
import LoadingPanel from "@/components/LoadingPanel.vue";
import TimezonePicker from "@/components/TimezonePicker.vue";

const router = useRouter();

const schedules = ref<Schedule[]>([]);
const loading = ref(false);
const error = ref("");

const searchQuery = ref("");

const filteredSchedules = computed(() => {
  const q = searchQuery.value.toLowerCase().trim();
  if (!q) return schedules.value;
  return schedules.value.filter((s) => s.name.toLowerCase().includes(q));
});

// Create form state
const showForm = ref(false);
const formName = ref("");
const formCron = ref("");
const formPrompt = ref("");
const formTimezone = ref("UTC");
const formSources = ref("");
const formEnabled = ref(true);
const formMissedPolicy = ref<"skip" | "run_once" | "catch_up">("run_once");
const formDigestMode = ref<"full" | "changes_only">("full");
const formMaxConcurrency = ref(1);
const formMaxCatchupRuns = ref(5);
const formSubmitting = ref(false);
const formError = ref("");

// Delete confirmation
const confirmDeleteId = ref<string | null>(null);
const deleting = ref(false);

// Polling
let pollTimer: ReturnType<typeof setInterval> | null = null;

async function load() {
  loading.value = true;
  error.value = "";
  try {
    const res = await api.getSchedules();
    schedules.value = res.schedules;
  } catch (e: unknown) {
    error.value = e instanceof ApiError ? e.friendly : String(e);
  } finally {
    loading.value = false;
  }
}

function startPolling() {
  stopPolling();
  pollTimer = setInterval(load, 10000);
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

// ── Create schedule ─────────────────────────────────────────────

function openForm() {
  showForm.value = true;
  formName.value = "";
  formCron.value = "";
  formTimezone.value = "UTC";
  formPrompt.value = "";
  formSources.value = "";
  formEnabled.value = true;
  formMissedPolicy.value = "run_once";
  formDigestMode.value = "full";
  formMaxConcurrency.value = 1;
  formMaxCatchupRuns.value = 5;
  formError.value = "";
}

function cancelForm() {
  showForm.value = false;
  formError.value = "";
}

async function submitForm() {
  formError.value = "";
  if (!formName.value.trim() || !formCron.value.trim() || !formPrompt.value.trim()) {
    formError.value = "Name, cron expression, and prompt template are required.";
    return;
  }

  const sources = formSources.value
    .split("\n")
    .map((s) => s.trim())
    .filter((s) => s.length > 0);

  const req: CreateScheduleRequest = {
    name: formName.value.trim(),
    cron: formCron.value.trim(),
    timezone: formTimezone.value,
    prompt_template: formPrompt.value.trim(),
    sources: sources.length > 0 ? sources : undefined,
    enabled: formEnabled.value,
    missed_policy: formMissedPolicy.value,
    digest_mode: formDigestMode.value,
    max_concurrency: formMaxConcurrency.value,
    max_catchup_runs: formMaxCatchupRuns.value,
  };

  formSubmitting.value = true;
  try {
    await api.createSchedule(req);
    showForm.value = false;
    await load();
  } catch (e: unknown) {
    formError.value = e instanceof ApiError ? e.friendly : String(e);
  } finally {
    formSubmitting.value = false;
  }
}

// ── Toggle enabled ──────────────────────────────────────────────

async function toggleEnabled(schedule: Schedule) {
  try {
    await api.updateSchedule(schedule.id, { enabled: !schedule.enabled });
    await load();
  } catch (e: unknown) {
    error.value = e instanceof ApiError ? e.friendly : String(e);
  }
}

// ── Run now ─────────────────────────────────────────────────────

async function runNow(schedule: Schedule, event: Event) {
  event.stopPropagation();
  try {
    await api.runScheduleNow(schedule.id);
    await load();
  } catch (e: unknown) {
    error.value = e instanceof ApiError ? e.friendly : String(e);
  }
}

// ── Delete ──────────────────────────────────────────────────────

function promptDelete(id: string, event: Event) {
  event.stopPropagation();
  confirmDeleteId.value = id;
}

function cancelDelete() {
  confirmDeleteId.value = null;
}

async function confirmDelete(id: string, event: Event) {
  event.stopPropagation();
  deleting.value = true;
  try {
    await api.deleteSchedule(id);
    confirmDeleteId.value = null;
    await load();
  } catch (e: unknown) {
    error.value = e instanceof ApiError ? e.friendly : String(e);
  } finally {
    deleting.value = false;
  }
}

// ── Helpers ─────────────────────────────────────────────────────

function timeAgo(iso?: string): string {
  if (!iso) return "-";
  const secs = Math.floor((Date.now() - new Date(iso).getTime()) / 1000);
  if (secs < 0) return formatFuture(-secs);
  if (secs < 60) return `${secs}s ago`;
  if (secs < 3600) return `${Math.floor(secs / 60)}m ago`;
  if (secs < 86400) return `${Math.floor(secs / 3600)}h ago`;
  return `${Math.floor(secs / 86400)}d ago`;
}

function formatFuture(secs: number): string {
  if (secs < 60) return `in ${secs}s`;
  if (secs < 3600) return `in ${Math.floor(secs / 60)}m`;
  if (secs < 86400) return `in ${Math.floor(secs / 3600)}h`;
  return `in ${Math.floor(secs / 86400)}d`;
}

function formatNextRun(iso?: string): string {
  if (!iso) return "-";
  const secs = Math.floor((new Date(iso).getTime() - Date.now()) / 1000);
  if (secs < 0) return "overdue";
  return formatFuture(secs);
}

function statusLabel(schedule: Schedule): string {
  if (schedule.status === "error") return "Error";
  if (schedule.status === "paused") return "Paused";
  return "Active";
}

function statusClass(schedule: Schedule): string {
  if (schedule.status === "error") return "status-error";
  if (schedule.enabled) return "status-enabled";
  return "status-paused";
}

function goToSchedule(id: string) {
  // Navigate to schedule detail if route exists, otherwise no-op
  router.push(`/schedules/${id}`).catch(() => {});
}
</script>

<template>
  <div>
    <h1 class="page-title">Schedules</h1>

    <!-- Filter bar -->
    <div class="filter-bar">
      <button class="create-btn" @click="openForm">+ Create Schedule</button>
      <input
        v-model="searchQuery"
        class="search-input"
        type="text"
        placeholder="Filter by name..."
      />
      <button class="refresh-btn" @click="load" :disabled="loading">Refresh</button>
      <span class="total dim">{{ filteredSchedules.length }} of {{ schedules.length }} total</span>
    </div>

    <p v-if="error" class="error">{{ error }}</p>

    <!-- Inline create form -->
    <Card v-if="showForm" title="New Schedule">
      <p v-if="formError" class="error">{{ formError }}</p>

      <div class="field">
        <label>Name</label>
        <input v-model="formName" placeholder="My daily digest" />
      </div>

      <div class="field">
        <label>Cron Expression</label>
        <input v-model="formCron" placeholder="*/30 * * * *" />
      </div>

      <div class="field">
        <label>Timezone</label>
        <TimezonePicker v-model="formTimezone" />
      </div>

      <div class="field">
        <label>Prompt Template</label>
        <textarea v-model="formPrompt" rows="4" placeholder="Summarize the latest updates from the following sources..."></textarea>
      </div>

      <div class="field">
        <label>URLs / Sources (one per line)</label>
        <textarea v-model="formSources" rows="3" placeholder="https://example.com/feed&#10;https://another.com/api"></textarea>
      </div>

      <div class="field-row">
        <div class="field">
          <label>Missed Policy</label>
          <select v-model="formMissedPolicy">
            <option value="run_once">Run Once</option>
            <option value="catch_up">Catch Up</option>
            <option value="skip">Skip</option>
          </select>
        </div>
        <div class="field">
          <label>Digest Mode</label>
          <select v-model="formDigestMode">
            <option value="full">Full</option>
            <option value="changes_only">Changes Only</option>
          </select>
        </div>
        <div class="field">
          <label>Max Concurrency</label>
          <input type="number" v-model.number="formMaxConcurrency" min="1" max="10" />
        </div>
        <div class="field">
          <label>Max Catch-up Runs</label>
          <input type="number" v-model.number="formMaxCatchupRuns" min="1" max="100" />
        </div>
      </div>

      <div class="field toggle-field">
        <label class="toggle-label">
          <input type="checkbox" v-model="formEnabled" />
          <span>Enabled</span>
        </label>
      </div>

      <div class="form-actions">
        <button @click="submitForm" :disabled="formSubmitting">
          {{ formSubmitting ? "Creating..." : "Create Schedule" }}
        </button>
        <button class="secondary" @click="cancelForm" :disabled="formSubmitting">Cancel</button>
      </div>
    </Card>

    <!-- Loading state -->
    <Card v-if="loading && schedules.length === 0" title="Loading">
      <LoadingPanel message="Loading schedules..." />
    </Card>

    <!-- Empty state -->
    <Card v-if="!loading && schedules.length === 0 && !error" title="No Schedules">
      <EmptyState
        icon="@"
        title="No schedules found"
        description="Create a schedule to run automated tasks on a cron-based cadence."
      />
    </Card>

    <!-- Schedules table -->
    <Card v-if="schedules.length > 0" title="All Schedules">
      <table class="tbl">
        <thead>
          <tr>
            <th>Name</th>
            <th>Cron</th>
            <th>Status</th>
            <th>Next Run</th>
            <th>Last Run</th>
            <th>Actions</th>
          </tr>
        </thead>
        <tbody>
          <tr
            v-for="s in filteredSchedules"
            :key="s.id"
            class="clickable"
            @click="goToSchedule(s.id)"
          >
            <td class="name-cell">{{ s.name }}</td>
            <td class="mono">{{ s.cron }}</td>
            <td>
              <span class="status-badge" :class="statusClass(s)">{{ statusLabel(s) }}</span>
              <span v-if="s.last_error" class="error-hint" :title="s.last_error">{{ s.consecutive_failures }}x</span>
            </td>
            <td class="dim">{{ formatNextRun(s.next_run_at) }}</td>
            <td class="dim">{{ timeAgo(s.last_run_at) }}</td>
            <td class="actions-cell" @click.stop>
              <button class="action-btn run-btn" @click="runNow(s, $event)" title="Run now">Run Now</button>
              <button
                class="action-btn toggle-btn"
                :class="s.enabled ? 'toggle-on' : 'toggle-off'"
                @click="toggleEnabled(s)"
                :title="s.enabled ? 'Pause schedule' : 'Enable schedule'"
              >{{ s.enabled ? "Pause" : "Enable" }}</button>
              <template v-if="confirmDeleteId === s.id">
                <button class="action-btn confirm-del-btn" @click="confirmDelete(s.id, $event)" :disabled="deleting">
                  {{ deleting ? "..." : "Confirm" }}
                </button>
                <button class="action-btn cancel-del-btn" @click.stop="cancelDelete">Cancel</button>
              </template>
              <button v-else class="action-btn del-btn" @click="promptDelete(s.id, $event)">Delete</button>
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
.mono { font-family: var(--mono); font-size: 0.82rem; }

/* ── Filter bar ─────────────────────────────────────────────── */
.filter-bar {
  display: flex;
  align-items: center;
  gap: 0.6rem;
  margin-bottom: 1rem;
}

.create-btn {
  background: var(--accent-dim);
  color: white;
  border: none;
  padding: 0.4rem 1rem;
  border-radius: 4px;
  font-size: 0.82rem;
  cursor: pointer;
}
.create-btn:hover { background: var(--accent); }

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

.search-input {
  background: var(--bg);
  border: 1px solid var(--border);
  color: var(--text);
  padding: 0.35rem 0.6rem;
  border-radius: 4px;
  font-size: 0.82rem;
  width: 180px;
}
.search-input::placeholder { color: var(--text-dim); }

.total { margin-left: auto; }

/* ── Form ───────────────────────────────────────────────────── */
.field-row {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(160px, 1fr));
  gap: 0.8rem;
  margin-bottom: 0.8rem;
}
.field-row .field { margin-bottom: 0; }

.field {
  display: flex;
  flex-direction: column;
  gap: 0.2rem;
  margin-bottom: 0.8rem;
}

.field label {
  font-size: 0.78rem;
  color: var(--text-dim);
}

.field input,
.field textarea,
.field select {
  background: var(--bg);
  border: 1px solid var(--border);
  color: var(--text);
  padding: 0.5rem 0.8rem;
  border-radius: 4px;
  font-family: var(--mono);
  font-size: 0.88rem;
  width: 100%;
  box-sizing: border-box;
}

.field textarea {
  resize: vertical;
  line-height: 1.5;
}

.toggle-field {
  flex-direction: row;
  align-items: center;
}

.toggle-label {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  font-size: 0.88rem !important;
  color: var(--text) !important;
  cursor: pointer;
}

.toggle-label input {
  width: auto !important;
  cursor: pointer;
}

.form-actions {
  display: flex;
  gap: 0.6rem;
  margin-top: 0.5rem;
}

.form-actions button {
  padding: 0.5rem 1.2rem;
  border-radius: 4px;
  font-size: 0.85rem;
  cursor: pointer;
}

.form-actions button:first-child {
  background: var(--accent-dim);
  color: white;
  border: none;
}
.form-actions button:first-child:hover:not(:disabled) { background: var(--accent); }
.form-actions button:first-child:disabled { opacity: 0.5; cursor: not-allowed; }

.form-actions button.secondary {
  background: transparent;
  border: 1px solid var(--border);
  color: var(--text-dim);
}
.form-actions button.secondary:hover:not(:disabled) { color: var(--text); border-color: var(--text-dim); }

/* ── Table ──────────────────────────────────────────────────── */
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

.name-cell {
  font-weight: 500;
  max-width: 200px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* ── Status badges ──────────────────────────────────────────── */
.status-badge {
  display: inline-block;
  padding: 0.15rem 0.5rem;
  border-radius: 3px;
  font-size: 0.78rem;
  font-weight: 500;
}

.status-enabled {
  background: rgba(63, 185, 80, 0.15);
  color: var(--green);
}

.status-paused {
  background: rgba(139, 148, 158, 0.15);
  color: var(--text-dim);
}

.status-error {
  background: rgba(248, 81, 73, 0.15);
  color: var(--red);
}

.error-hint {
  font-size: 0.72rem;
  color: var(--red);
  margin-left: 0.3rem;
  cursor: help;
}

/* ── Action buttons ─────────────────────────────────────────── */
.actions-cell {
  display: flex;
  gap: 0.4rem;
  align-items: center;
  flex-wrap: nowrap;
}

.action-btn {
  background: transparent;
  border: 1px solid var(--border);
  color: var(--text-dim);
  padding: 0.2rem 0.6rem;
  border-radius: 3px;
  font-size: 0.75rem;
  cursor: pointer;
  white-space: nowrap;
}
.action-btn:hover:not(:disabled) { color: var(--text); border-color: var(--text-dim); }
.action-btn:disabled { opacity: 0.5; cursor: not-allowed; }

.run-btn { color: var(--accent); border-color: var(--accent); }
.run-btn:hover { background: rgba(88, 166, 255, 0.1); }

.toggle-on { color: var(--text-dim); }
.toggle-off { color: var(--green); border-color: var(--green); }
.toggle-off:hover { background: rgba(63, 185, 80, 0.1); }

.del-btn { color: var(--red); border-color: transparent; }
.del-btn:hover { border-color: var(--red); background: rgba(248, 81, 73, 0.1); }

.confirm-del-btn { color: var(--red); border-color: var(--red); background: rgba(248, 81, 73, 0.1); }
.cancel-del-btn { color: var(--text-dim); }
</style>
