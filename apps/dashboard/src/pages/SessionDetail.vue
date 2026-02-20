<script setup lang="ts">
import { ref, onMounted, computed } from "vue";
import { api, buildHeaders } from "@/api/client";
import type { SessionDetailResponse, TranscriptLine } from "@/api/client";
import Card from "@/components/Card.vue";
import StatusDot from "@/components/StatusDot.vue";

const props = defineProps<{ key: string }>();

const session = ref<SessionDetailResponse | null>(null);
const lines = ref<TranscriptLine[]>([]);
const totalLines = ref(0);
const error = ref("");
const offset = ref(0);
const limit = 200;
const copied = ref(false);
const actionMsg = ref("");
const exportOpen = ref(false);

const sessionKey = computed(() => props.key);

async function load() {
  try {
    session.value = await api.session(sessionKey.value);
    const res = await api.transcript(sessionKey.value, offset.value, limit);
    lines.value = res.lines;
    totalLines.value = res.total;
  } catch (e: any) {
    error.value = e.message;
  }
}

function roleColor(role: string): string {
  switch (role) {
    case "user": return "var(--accent)";
    case "assistant": return "var(--green)";
    case "tool": return "var(--yellow)";
    case "system": return "var(--text-dim)";
    default: return "var(--text)";
  }
}

function isCompactionMarker(line: TranscriptLine): boolean {
  if (line.role === "system" && line.metadata) {
    return !!(line.metadata as any)["sa.compaction"];
  }
  return false;
}

function copyKey() {
  navigator.clipboard.writeText(sessionKey.value).then(() => {
    copied.value = true;
    setTimeout(() => { copied.value = false; }, 1500);
  });
}

async function stopSession() {
  try {
    const res = await api.stopSession(sessionKey.value);
    actionMsg.value = res.stopped ? "Turn cancelled" : "No running turn";
    await load();
    setTimeout(() => { actionMsg.value = ""; }, 2000);
  } catch (e: any) {
    error.value = e.message;
  }
}

async function resetSession() {
  try {
    await api.resetSession(sessionKey.value);
    actionMsg.value = "Session reset";
    await load();
    setTimeout(() => { actionMsg.value = ""; }, 2000);
  } catch (e: any) {
    error.value = e.message;
  }
}

async function exportTranscript(format: string) {
  exportOpen.value = false;
  try {
    const url = `/v1/sessions/${encodeURIComponent(sessionKey.value)}/export?format=${format}`;
    const res = await fetch(url, { headers: buildHeaders() });
    if (!res.ok) {
      error.value = `Export failed: ${res.status}`;
      return;
    }
    const blob = await res.blob();
    const ext = format === "markdown" ? "md" : format === "jsonl" ? "jsonl" : "json";
    const filename = `session-${sessionKey.value}.${ext}`;
    const a = document.createElement("a");
    a.href = URL.createObjectURL(blob);
    a.download = filename;
    a.click();
    URL.revokeObjectURL(a.href);
  } catch (e: any) {
    error.value = e.message;
  }
}

function nextPage() {
  offset.value += limit;
  load();
}
function prevPage() {
  offset.value = Math.max(0, offset.value - limit);
  load();
}

onMounted(load);
</script>

<template>
  <div>
    <h1 class="page-title">
      <router-link to="/sessions" class="back">&larr;</router-link>
      Session Detail
    </h1>
    <p v-if="error" class="error">{{ error }}</p>
    <p v-if="actionMsg" class="action-msg">{{ actionMsg }}</p>

    <Card v-if="session" title="Metadata">
      <div class="detail-grid">
        <div>
          <span class="label">Key</span>
          <code>{{ session.session_key }}</code>
          <button class="copy-btn" :class="{ ok: copied }" @click="copyKey">
            {{ copied ? "copied" : "copy" }}
          </button>
        </div>
        <div><span class="label">ID</span><code class="dim">{{ session.session_id }}</code></div>
        <div>
          <span class="label">Status</span>
          <StatusDot :status="session.running ? 'ok' : 'off'" />
          {{ session.running ? "Running" : "Idle" }}
        </div>
        <div><span class="label">Model</span><code>{{ session.model || "default" }}</code></div>
        <div><span class="label">Created</span><span class="dim">{{ new Date(session.created_at).toLocaleString() }}</span></div>
        <div><span class="label">Updated</span><span class="dim">{{ new Date(session.updated_at).toLocaleString() }}</span></div>
        <div v-if="session.origin?.channel"><span class="label">Channel</span>{{ session.origin.channel }}</div>
        <div v-if="session.origin?.peer"><span class="label">Peer</span>{{ session.origin.peer }}</div>
        <div><span class="label">Tokens</span>in={{ session.tokens?.input?.toLocaleString() }} out={{ session.tokens?.output?.toLocaleString() }} total={{ session.tokens?.total?.toLocaleString() }}</div>
      </div>

      <div class="action-bar">
        <button v-if="session.running" class="action-btn stop" @click="stopSession">Stop Turn</button>
        <button class="action-btn reset" @click="resetSession">Reset Session</button>
        <div class="export-wrap">
          <button class="action-btn export" @click="exportOpen = !exportOpen">Export</button>
          <div v-if="exportOpen" class="export-menu">
            <button class="export-item" @click="exportTranscript('markdown')">Markdown</button>
            <button class="export-item" @click="exportTranscript('jsonl')">JSONL</button>
          </div>
        </div>
      </div>
    </Card>

    <Card :title="`Transcript (${totalLines} lines)`">
      <div v-if="lines.length === 0" class="dim" style="text-align:center;padding:2rem">
        No transcript lines
      </div>
      <div v-else class="transcript">
        <div
          v-for="(line, i) in lines"
          :key="i"
          class="line"
          :class="{ compaction: isCompactionMarker(line) }"
        >
          <span class="ts dim">{{ new Date(line.timestamp).toLocaleTimeString() }}</span>
          <span class="role" :style="{ color: roleColor(line.role) }">{{ line.role }}</span>
          <span class="content">{{ line.content }}</span>
        </div>
      </div>

      <div v-if="totalLines > limit" class="paging">
        <button class="page-btn" :disabled="offset === 0" @click="prevPage">Prev</button>
        <span class="dim">{{ offset + 1 }}&ndash;{{ Math.min(offset + lines.length, totalLines) }} of {{ totalLines }}</span>
        <button class="page-btn" :disabled="offset + limit >= totalLines" @click="nextPage">Next</button>
      </div>
    </Card>
  </div>
</template>

<style scoped>
.page-title { font-size: 1.5rem; margin-bottom: 1.5rem; color: var(--accent); }
.back { margin-right: 0.5em; color: var(--text-dim); text-decoration: none; }
.back:hover { color: var(--accent); }
.error { color: var(--red); margin-bottom: 1rem; }
.action-msg { color: var(--green); margin-bottom: 1rem; font-size: 0.88rem; }
.dim { color: var(--text-dim); }
.detail-grid { display: grid; grid-template-columns: 1fr 1fr; gap: 0.5rem 2rem; font-size: 0.88rem; }
.label { color: var(--text-dim); margin-right: 0.5em; font-size: 0.82rem; }

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
.copy-btn:hover { color: var(--text); }
.copy-btn.ok { color: var(--green); border-color: var(--green); }

.action-bar {
  display: flex;
  gap: 0.5rem;
  margin-top: 1rem;
}
.action-btn {
  background: transparent;
  border: 1px solid var(--border);
  color: var(--text-dim);
  padding: 0.35rem 0.8rem;
  border-radius: 4px;
  cursor: pointer;
  font-size: 0.82rem;
}
.action-btn:hover { color: var(--text); }
.action-btn.stop { border-color: var(--red); color: var(--red); }
.action-btn.stop:hover { background: rgba(248, 81, 73, 0.1); }
.action-btn.reset { border-color: var(--yellow); color: var(--yellow); }
.action-btn.reset:hover { background: rgba(210, 153, 34, 0.1); }
.action-btn.export { border-color: var(--accent); color: var(--accent); }
.action-btn.export:hover { background: rgba(88, 166, 255, 0.1); }

.export-wrap { position: relative; }
.export-menu {
  position: absolute;
  top: 100%;
  left: 0;
  margin-top: 0.3rem;
  background: #161b22;
  border: 1px solid var(--border);
  border-radius: 4px;
  z-index: 10;
  min-width: 8rem;
}
.export-item {
  display: block;
  width: 100%;
  background: transparent;
  border: none;
  color: var(--text-dim);
  padding: 0.4rem 0.8rem;
  text-align: left;
  cursor: pointer;
  font-size: 0.82rem;
}
.export-item:hover { color: var(--text); background: rgba(88, 166, 255, 0.08); }

.transcript { display: flex; flex-direction: column; gap: 0; }
.line {
  display: flex;
  gap: 0.6rem;
  padding: 0.35rem 0.4rem;
  font-size: 0.82rem;
  border-bottom: 1px solid rgba(48, 54, 61, 0.5);
  align-items: flex-start;
}
.line:hover { background: rgba(88, 166, 255, 0.03); }
.ts { flex-shrink: 0; width: 5.5em; font-family: var(--mono); font-size: 0.75rem; }
.role { flex-shrink: 0; width: 5em; font-weight: 600; font-size: 0.78rem; text-transform: uppercase; }
.content { white-space: pre-wrap; word-break: break-word; flex: 1; }
.compaction {
  background: rgba(210, 153, 34, 0.08);
  border-left: 2px solid var(--yellow);
}
.paging {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 0.8rem;
  margin-top: 0.8rem;
  font-size: 0.82rem;
}
.page-btn {
  background: #21262d;
  border: 1px solid var(--border);
  color: var(--text-dim);
  padding: 0.25rem 0.8rem;
  border-radius: 3px;
  cursor: pointer;
  font-size: 0.8rem;
}
.page-btn:hover:not(:disabled) { color: var(--text); }
.page-btn:disabled { opacity: 0.4; cursor: not-allowed; }
</style>
