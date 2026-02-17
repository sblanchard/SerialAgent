<script setup lang="ts">
import { ref, onMounted, computed } from "vue";
import { api, ApiError } from "@/api/client";
import type { StagingEntry } from "@/api/client";
import Card from "@/components/Card.vue";
import StatusDot from "@/components/StatusDot.vue";

const entries = ref<StagingEntry[]>([]);
const loading = ref(false);
const error = ref("");

// Delete confirmation
const confirmDeleteId = ref<string | null>(null);
const deleting = ref(false);

async function load() {
  loading.value = true;
  error.value = "";
  try {
    const res = await api.listStaging();
    entries.value = res.entries;
  } catch (e: unknown) {
    error.value = e instanceof ApiError ? e.friendly : String(e);
  } finally {
    loading.value = false;
  }
}

async function deleteEntry(id: string) {
  deleting.value = true;
  try {
    await api.deleteStaging(id);
    entries.value = entries.value.filter((e) => e.id !== id);
    confirmDeleteId.value = null;
  } catch (e: unknown) {
    // "not found" is non-fatal — remove from list
    if (e instanceof ApiError && e.status === 404) {
      entries.value = entries.value.filter((e) => e.id !== id);
      confirmDeleteId.value = null;
    } else {
      error.value = e instanceof ApiError ? e.friendly : String(e);
    }
  } finally {
    deleting.value = false;
  }
}

function formatAge(secs: number): string {
  if (secs < 60) return `${secs}s ago`;
  if (secs < 3600) return `${Math.floor(secs / 60)}m ago`;
  if (secs < 86400) return `${Math.floor(secs / 3600)}h ago`;
  return `${Math.floor(secs / 86400)}d ago`;
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

const isEmpty = computed(() => !loading.value && entries.value.length === 0);

onMounted(load);
</script>

<template>
  <div>
    <h1 class="page-title">Import Staging</h1>

    <p v-if="error" class="error">{{ error }}</p>

    <!-- Empty state -->
    <Card v-if="isEmpty" title="No Staging Data">
      <div class="empty">
        <p>No staging directories found. Staging data is created during import preview and automatically cleaned up after 24 hours.</p>
        <div class="action-bar">
          <router-link to="/import" class="action-link">Start an import</router-link>
        </div>
      </div>
    </Card>

    <!-- Loading -->
    <Card v-if="loading" title="Loading">
      <div class="loading-row">
        <div class="spinner"></div>
        <span class="dim">Loading staging entries...</span>
      </div>
    </Card>

    <!-- Staging table -->
    <Card v-if="!loading && entries.length > 0" title="Staging Entries">
      <div class="table-actions">
        <button class="secondary" @click="load" :disabled="loading">Refresh</button>
        <span class="dim">{{ entries.length }} entries</span>
      </div>

      <table class="tbl">
        <thead>
          <tr>
            <th>Staging ID</th>
            <th>Age</th>
            <th>Size</th>
            <th>Status</th>
            <th>Actions</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="entry in entries" :key="entry.id">
            <td><code class="staging-id">{{ entry.id }}</code></td>
            <td class="dim">{{ formatAge(entry.age_secs) }}</td>
            <td>{{ formatBytes(entry.size_bytes) }}</td>
            <td>
              <StatusDot :status="entry.has_extracted ? 'ok' : 'warn'" />
              {{ entry.has_extracted ? "Extracted" : "Pending" }}
            </td>
            <td>
              <template v-if="confirmDeleteId === entry.id">
                <span class="confirm-row">
                  <button class="danger-btn" @click="deleteEntry(entry.id)" :disabled="deleting">
                    {{ deleting ? "Deleting..." : "Confirm Delete" }}
                  </button>
                  <button class="secondary cancel-btn" @click="confirmDeleteId = null" :disabled="deleting">Cancel</button>
                </span>
              </template>
              <button v-else class="secondary delete-btn" @click="confirmDeleteId = entry.id">Delete</button>
            </td>
          </tr>
        </tbody>
      </table>
    </Card>

    <div class="page-footer">
      <router-link to="/import" class="secondary-link">Back to Import</router-link>
    </div>
  </div>
</template>

<style scoped>
.page-title { font-size: 1.5rem; margin-bottom: 1.5rem; color: var(--accent); }
.error { color: var(--red); margin-bottom: 1rem; }
.dim { color: var(--text-dim); font-size: 0.85rem; }

.empty {
  text-align: center;
  padding: 1.5rem 0;
  font-size: 0.9rem;
  color: var(--text-dim);
}
.action-link {
  font-size: 0.85rem;
}
.action-bar { display: flex; gap: 0.6rem; align-items: center; justify-content: center; margin-top: 1rem; }

.loading-row {
  display: flex;
  align-items: center;
  gap: 0.8rem;
  padding: 1rem 0;
}
.spinner {
  width: 20px;
  height: 20px;
  border: 3px solid var(--border);
  border-top-color: var(--accent);
  border-radius: 50%;
  animation: spin 0.8s linear infinite;
  flex-shrink: 0;
}
@keyframes spin { to { transform: rotate(360deg); } }

.table-actions {
  display: flex;
  align-items: center;
  gap: 0.8rem;
  margin-bottom: 0.8rem;
}

.tbl { width: 100%; border-collapse: collapse; font-size: 0.85rem; }
.tbl th { color: var(--text-dim); font-weight: 600; text-align: left; padding: 0.5rem 0.6rem; border-bottom: 1px solid var(--border); }
.tbl td { padding: 0.5rem 0.6rem; border-bottom: 1px solid var(--border); vertical-align: middle; }

.staging-id { font-size: 0.75rem; }

button {
  background: var(--accent-dim);
  color: white;
  border: none;
  padding: 0.35rem 0.8rem;
  border-radius: 4px;
  cursor: pointer;
  font-size: 0.78rem;
}
button:disabled { opacity: 0.5; cursor: not-allowed; }
button:hover:not(:disabled) { background: var(--accent); }
button.secondary {
  background: transparent;
  border: 1px solid var(--border);
  color: var(--text-dim);
}
button.secondary:hover:not(:disabled) { color: var(--text); border-color: var(--text-dim); }

.delete-btn { font-size: 0.78rem; }
.danger-btn {
  background: var(--red);
  font-size: 0.78rem;
}
.danger-btn:hover:not(:disabled) { background: #da3633; }
.cancel-btn { font-size: 0.78rem; }

.confirm-row {
  display: flex;
  gap: 0.4rem;
  align-items: center;
}

.page-footer {
  margin-top: 1.5rem;
}
.secondary-link { font-size: 0.85rem; }
</style>
