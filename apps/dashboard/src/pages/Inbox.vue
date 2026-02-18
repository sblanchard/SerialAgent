<script setup lang="ts">
import { ref, onMounted, onUnmounted, computed, watch } from "vue";
import { useRouter } from "vue-router";
import { api, ApiError } from "@/api/client";
import type { Delivery } from "@/api/client";

const router = useRouter();
import { subscribeSSE } from "@/api/sse";
import Card from "@/components/Card.vue";
import EmptyState from "@/components/EmptyState.vue";
import LoadingPanel from "@/components/LoadingPanel.vue";

const deliveries = ref<Delivery[]>([]);
const total = ref(0);
const unread = ref(0);
const loading = ref(false);
const error = ref("");

// Filters
const readFilter = ref<"" | "unread" | "read">("");
const page = ref(0);
const pageSize = 25;

// Expanded delivery
const expandedId = ref<string | null>(null);

// Polling
let pollTimer: ReturnType<typeof setInterval> | null = null;

// SSE subscription cleanup
let unsub: (() => void) | null = null;

async function load() {
  loading.value = true;
  error.value = "";
  try {
    const res = await api.getDeliveries(pageSize, page.value * pageSize);
    deliveries.value = res.deliveries;
    total.value = res.total;
    unread.value = res.unread;
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

function startSSE() {
  stopSSE();
  unsub = subscribeSSE("/v1/deliveries/events", {
    onEvent(_type, _data) {
      // Refresh the list when a new delivery event arrives
      load();
    },
    onError() {
      // SSE failed; polling covers us as a fallback
    },
  });
}

function stopSSE() {
  if (unsub) {
    unsub();
    unsub = null;
  }
}

async function toggleExpand(delivery: Delivery) {
  if (expandedId.value === delivery.id) {
    expandedId.value = null;
    return;
  }
  expandedId.value = delivery.id;
  if (!delivery.read) {
    await markRead(delivery);
  }
}

async function markRead(delivery: Delivery) {
  try {
    await api.markDeliveryRead(delivery.id);
    delivery.read = true;
    unread.value = Math.max(0, unread.value - 1);
  } catch {
    // Silent failure; will reconcile on next poll
  }
}

const filteredDeliveries = computed(() => {
  if (!readFilter.value) return deliveries.value;
  if (readFilter.value === "unread") return deliveries.value.filter((d) => !d.read);
  return deliveries.value.filter((d) => d.read);
});

const hasMore = computed(() => (page.value + 1) * pageSize < total.value);

function timeAgo(iso: string): string {
  const secs = Math.floor((Date.now() - new Date(iso).getTime()) / 1000);
  if (secs < 60) return `${secs}s ago`;
  if (secs < 3600) return `${Math.floor(secs / 60)}m ago`;
  if (secs < 86400) return `${Math.floor(secs / 3600)}h ago`;
  return `${Math.floor(secs / 86400)}d ago`;
}

function formatDate(iso: string): string {
  const d = new Date(iso);
  return d.toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

watch(readFilter, () => {
  page.value = 0;
});

watch(page, load);

onMounted(() => {
  load();
  startPolling();
  startSSE();
});

onUnmounted(() => {
  stopPolling();
  stopSSE();
});
</script>

<template>
  <div>
    <h1 class="page-title">
      Inbox
      <span v-if="unread > 0" class="unread-badge">{{ unread }}</span>
    </h1>

    <!-- Controls bar -->
    <div class="filter-bar">
      <select v-model="readFilter" class="filter-select">
        <option value="">All deliveries</option>
        <option value="unread">Unread</option>
        <option value="read">Read</option>
      </select>
      <button class="refresh-btn" @click="load" :disabled="loading">Refresh</button>
      <span class="total dim">{{ total }} total</span>
    </div>

    <p v-if="error" class="error">{{ error }}</p>

    <Card v-if="loading && deliveries.length === 0" title="Loading">
      <LoadingPanel message="Loading deliveries..." />
    </Card>

    <Card v-if="!loading && filteredDeliveries.length === 0 && !error" title="No Deliveries">
      <EmptyState
        icon="@"
        title="No deliveries found"
        description="Deliveries appear here when scheduled runs complete. Create a schedule with an in-app delivery target to start receiving content."
      />
    </Card>

    <Card v-if="filteredDeliveries.length > 0" title="Deliveries">
      <table class="tbl">
        <thead>
          <tr>
            <th class="col-status"></th>
            <th>Title</th>
            <th>Schedule</th>
            <th>Created</th>
          </tr>
        </thead>
        <tbody>
          <template v-for="delivery in filteredDeliveries" :key="delivery.id">
            <tr
              class="clickable"
              :class="{ 'row-unread': !delivery.read, 'row-expanded': expandedId === delivery.id }"
              @click="toggleExpand(delivery)"
            >
              <td class="col-status">
                <span v-if="!delivery.read" class="unread-dot" title="Unread"></span>
              </td>
              <td class="col-title">
                <span :class="{ 'title-unread': !delivery.read }">{{ delivery.title }}</span>
              </td>
              <td class="col-schedule dim">{{ delivery.schedule_name || "-" }}</td>
              <td class="col-date">
                <span class="dim" :title="formatDate(delivery.created_at)">{{ timeAgo(delivery.created_at) }}</span>
              </td>
            </tr>
            <tr v-if="expandedId === delivery.id" class="detail-row">
              <td colspan="4">
                <div class="detail-panel">
                  <div class="detail-meta">
                    <span class="meta-label">Created:</span>
                    <span class="mono">{{ formatDate(delivery.created_at) }}</span>
                    <span v-if="delivery.schedule_id" class="meta-sep">|</span>
                    <span v-if="delivery.schedule_id" class="meta-label">Schedule:</span>
                    <a v-if="delivery.schedule_id" class="meta-link" @click.stop="router.push(`/schedules/${delivery.schedule_id}`)">{{ delivery.schedule_name }}</a>
                    <span v-if="delivery.run_id" class="meta-sep">|</span>
                    <span v-if="delivery.run_id" class="meta-label">Run:</span>
                    <a v-if="delivery.run_id" class="meta-link mono" @click.stop="router.push(`/runs/${delivery.run_id}`)">{{ delivery.run_id.slice(0, 8) }}</a>
                    <span v-if="delivery.total_tokens > 0" class="meta-sep">|</span>
                    <span v-if="delivery.total_tokens > 0" class="meta-label">Tokens:</span>
                    <span v-if="delivery.total_tokens > 0" class="mono">{{ delivery.total_tokens }}</span>
                  </div>
                  <div v-if="delivery.sources && delivery.sources.length > 0" class="detail-sources">
                    <span class="meta-label">Sources:</span>
                    <span v-for="(src, i) in delivery.sources" :key="i" class="source-tag">{{ src }}</span>
                  </div>
                  <div class="detail-body">{{ delivery.body }}</div>
                </div>
              </td>
            </tr>
          </template>
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
.page-title {
  font-size: 1.5rem;
  margin-bottom: 1.5rem;
  color: var(--accent);
  display: flex;
  align-items: center;
  gap: 0.6rem;
}

.unread-badge {
  background: var(--accent);
  color: var(--bg);
  font-size: 0.7rem;
  font-weight: 700;
  padding: 0.15rem 0.5rem;
  border-radius: 10px;
  min-width: 1.4rem;
  text-align: center;
  line-height: 1.3;
}

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

.row-unread { background: rgba(88, 166, 255, 0.02); }
.row-expanded { background: rgba(88, 166, 255, 0.04); }

.col-status { width: 1.5rem; text-align: center; }

.unread-dot {
  display: inline-block;
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: var(--accent);
}

.col-title {
  max-width: 400px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 0.85rem;
}

.title-unread { font-weight: 600; color: var(--text); }

.col-schedule {
  max-width: 180px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 0.82rem;
}

.col-date { white-space: nowrap; font-size: 0.82rem; }

.detail-row td {
  padding: 0;
  border-bottom: 1px solid var(--border);
}

.detail-panel {
  background: var(--bg);
  border-top: 1px solid var(--border);
  padding: 1rem 1.2rem;
}

.detail-meta {
  display: flex;
  align-items: center;
  gap: 0.4rem;
  flex-wrap: wrap;
  margin-bottom: 0.6rem;
  font-size: 0.82rem;
  color: var(--text-dim);
}

.meta-label { font-weight: 600; color: var(--text-dim); }
.meta-link { color: var(--accent); cursor: pointer; text-decoration: none; }
.meta-link:hover { text-decoration: underline; }
.meta-sep { color: var(--border); margin: 0 0.2rem; }

.detail-sources {
  display: flex;
  align-items: center;
  gap: 0.4rem;
  flex-wrap: wrap;
  margin-bottom: 0.8rem;
  font-size: 0.82rem;
}

.source-tag {
  background: var(--bg-card);
  border: 1px solid var(--border);
  color: var(--text-dim);
  padding: 0.1rem 0.5rem;
  border-radius: 3px;
  font-family: var(--mono);
  font-size: 0.75rem;
}

.detail-body {
  white-space: pre-wrap;
  word-break: break-word;
  font-size: 0.85rem;
  line-height: 1.6;
  color: var(--text);
  font-family: var(--mono);
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: 4px;
  padding: 0.8rem 1rem;
  max-height: 500px;
  overflow-y: auto;
}

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
