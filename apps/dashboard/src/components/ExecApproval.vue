<script setup lang="ts">
import { ref, onMounted, onUnmounted } from "vue";

interface PendingApproval {
  id: string;
  command: string;
  session_key: string;
  created_at: string;
}

const pending = ref<PendingApproval[]>([]);
const resolving = ref<Set<string>>(new Set());

const apiBase = import.meta.env.VITE_API_BASE ?? "";

async function fetchPending() {
  try {
    const res = await fetch(`${apiBase}/v1/tools/exec/pending`);
    if (res.ok) {
      const data = await res.json();
      pending.value = data.pending ?? [];
    }
  } catch {
    // Silently ignore — dashboard may poll before server is ready.
  }
}

async function approve(id: string) {
  resolving.value = new Set([...resolving.value, id]);
  try {
    await fetch(`${apiBase}/v1/tools/exec/approve/${id}`, { method: "POST" });
    pending.value = pending.value.filter((p) => p.id !== id);
  } finally {
    const next = new Set(resolving.value);
    next.delete(id);
    resolving.value = next;
  }
}

async function deny(id: string) {
  resolving.value = new Set([...resolving.value, id]);
  try {
    await fetch(`${apiBase}/v1/tools/exec/deny/${id}`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ reason: "Denied from dashboard" }),
    });
    pending.value = pending.value.filter((p) => p.id !== id);
  } finally {
    const next = new Set(resolving.value);
    next.delete(id);
    resolving.value = next;
  }
}

function handleSseEvent(event: MessageEvent) {
  try {
    const data = JSON.parse(event.data);
    if (data.type === "exec.approval_required") {
      pending.value = [
        ...pending.value,
        {
          id: data.approval_id,
          command: data.command,
          session_key: data.session_key,
          created_at: new Date().toISOString(),
        },
      ];
    }
  } catch {
    // Ignore malformed SSE data.
  }
}

let pollInterval: ReturnType<typeof setInterval> | undefined;

onMounted(() => {
  fetchPending();
  // Poll every 5 seconds as a fallback alongside SSE.
  pollInterval = setInterval(fetchPending, 5000);

  // Listen for SSE approval events on the global event bus if available.
  window.addEventListener("sa:run-event", handleSseEvent as EventListener);
});

onUnmounted(() => {
  if (pollInterval) {
    clearInterval(pollInterval);
  }
  window.removeEventListener("sa:run-event", handleSseEvent as EventListener);
});
</script>

<template>
  <div v-if="pending.length > 0" class="approval-overlay">
    <div
      v-for="item in pending"
      :key="item.id"
      class="approval-dialog"
    >
      <h3 class="approval-title">Exec Approval Required</h3>
      <p class="approval-session">
        Session: <code>{{ item.session_key }}</code>
      </p>
      <pre class="approval-command">{{ item.command }}</pre>
      <div class="approval-actions">
        <button
          class="btn btn-approve"
          :disabled="resolving.has(item.id)"
          @click="approve(item.id)"
        >
          Approve
        </button>
        <button
          class="btn btn-deny"
          :disabled="resolving.has(item.id)"
          @click="deny(item.id)"
        >
          Deny
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.approval-overlay {
  position: fixed;
  top: 0;
  left: 0;
  width: 100%;
  height: 100%;
  background: rgba(0, 0, 0, 0.5);
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  z-index: 9999;
  gap: 1rem;
}

.approval-dialog {
  background: var(--bg-card, #1e1e2e);
  border: 1px solid var(--border, #444);
  border-radius: 8px;
  padding: 1.5rem;
  max-width: 600px;
  width: 90%;
  box-shadow: 0 8px 32px rgba(0, 0, 0, 0.4);
}

.approval-title {
  color: var(--warning, #f9a825);
  font-size: 1rem;
  font-weight: 700;
  margin: 0 0 0.5rem;
}

.approval-session {
  color: var(--text-muted, #aaa);
  font-size: 0.8rem;
  margin: 0 0 0.75rem;
}

.approval-session code {
  color: var(--accent, #7aa2f7);
}

.approval-command {
  background: var(--bg-surface, #111);
  border: 1px solid var(--border, #333);
  border-radius: 4px;
  padding: 0.75rem 1rem;
  color: var(--text, #ddd);
  font-family: "JetBrains Mono", "Fira Code", monospace;
  font-size: 0.85rem;
  white-space: pre-wrap;
  word-break: break-all;
  margin: 0 0 1rem;
  max-height: 200px;
  overflow-y: auto;
}

.approval-actions {
  display: flex;
  gap: 0.75rem;
  justify-content: flex-end;
}

.btn {
  padding: 0.5rem 1.25rem;
  border: none;
  border-radius: 4px;
  font-size: 0.85rem;
  font-weight: 600;
  cursor: pointer;
  transition: opacity 0.15s;
}

.btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.btn-approve {
  background: #2ea043;
  color: #fff;
}

.btn-approve:hover:not(:disabled) {
  background: #3fb950;
}

.btn-deny {
  background: #da3633;
  color: #fff;
}

.btn-deny:hover:not(:disabled) {
  background: #f85149;
}
</style>
