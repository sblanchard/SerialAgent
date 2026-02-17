<script setup lang="ts">
import { ref, onMounted, onUnmounted, computed } from "vue";
import { api, ApiError } from "@/api/client";
import { subscribeSSE } from "@/api/sse";
import type { RunDetail, RunNode } from "@/api/client";
import Card from "@/components/Card.vue";
import RunStatusBadge from "@/components/RunStatusBadge.vue";
import LoadingPanel from "@/components/LoadingPanel.vue";
import CodeAccordion from "@/components/CodeAccordion.vue";

const props = defineProps<{ id: string }>();

const run = ref<RunDetail | null>(null);
const loading = ref(true);
const error = ref("");
const selectedNode = ref<RunNode | null>(null);

let unsub: (() => void) | null = null;

async function load() {
  loading.value = true;
  error.value = "";
  try {
    run.value = await api.getRun(props.id);
  } catch (e: unknown) {
    error.value = e instanceof ApiError ? e.friendly : String(e);
  } finally {
    loading.value = false;
  }
}

function startSSE() {
  if (!run.value || run.value.status === "completed" || run.value.status === "failed" || run.value.status === "stopped") {
    return;
  }
  unsub = subscribeSSE(`/v1/runs/${props.id}/events`, {
    onEvent(type, data: Record<string, unknown>) {
      if (type === "run.status") {
        // Reload full run data when status changes
        load();
      } else if (type === "node.started" || type === "node.completed" || type === "node.failed") {
        // Reload to get updated nodes
        load();
      } else if (type === "usage") {
        // Reload to get updated token counts
        load();
      }
    },
    onClose() {
      // Stream ended, do a final reload
      load();
    },
  });
}

onMounted(async () => {
  await load();
  startSSE();
});

onUnmounted(() => {
  if (unsub) unsub();
});

// Derived
const isLive = computed(() =>
  run.value && (run.value.status === "running" || run.value.status === "queued")
);

const llmNodes = computed(() =>
  (run.value?.nodes || []).filter((n) => n.kind === "llm_request")
);

const toolNodes = computed(() =>
  (run.value?.nodes || []).filter((n) => n.kind === "tool_call")
);

function formatDuration(ms?: number): string {
  if (ms == null) return "-";
  if (ms < 1000) return `${ms}ms`;
  return `${(ms / 1000).toFixed(1)}s`;
}

function formatTokens(n: number): string {
  if (n < 1000) return String(n);
  return `${(n / 1000).toFixed(1)}k`;
}

function formatTime(iso?: string): string {
  if (!iso) return "-";
  return new Date(iso).toLocaleTimeString();
}

function selectNode(node: RunNode) {
  selectedNode.value = selectedNode.value?.node_id === node.node_id ? null : node;
}
</script>

<template>
  <div>
    <div class="header-bar">
      <router-link to="/runs" class="back-link">Runs</router-link>
      <span class="sep">/</span>
      <span class="run-id mono">{{ id.substring(0, 8) }}...</span>
      <RunStatusBadge v-if="run" :status="run.status" />
      <span v-if="isLive" class="live-dot"></span>
    </div>

    <p v-if="error" class="error">{{ error }}</p>

    <Card v-if="loading && !run" title="Loading">
      <LoadingPanel message="Loading run..." />
    </Card>

    <template v-if="run">
      <!-- Run metadata -->
      <Card title="Run Summary">
        <div class="meta-grid">
          <div><span class="label">Run ID</span> <code class="mono val">{{ run.run_id }}</code></div>
          <div><span class="label">Session</span> <router-link :to="`/sessions/${run.session_key}`" class="link">{{ run.session_key }}</router-link></div>
          <div><span class="label">Status</span> <RunStatusBadge :status="run.status" size="sm" /></div>
          <div><span class="label">Duration</span> <span class="mono val">{{ formatDuration(run.duration_ms) }}</span></div>
          <div><span class="label">Agent</span> <span class="val">{{ run.agent_id || "default" }}</span></div>
          <div><span class="label">Model</span> <span class="mono val">{{ run.model || "default" }}</span></div>
          <div><span class="label">Tokens</span> <span class="mono val">{{ formatTokens(run.input_tokens) }} in / {{ formatTokens(run.output_tokens) }} out / {{ formatTokens(run.total_tokens) }} total</span></div>
          <div><span class="label">Tool Loops</span> <span class="mono val">{{ run.loop_count }}</span></div>
          <div><span class="label">Started</span> <span class="mono val">{{ formatTime(run.started_at) }}</span></div>
          <div v-if="run.ended_at"><span class="label">Ended</span> <span class="mono val">{{ formatTime(run.ended_at) }}</span></div>
        </div>

        <div v-if="run.error" class="error-block">
          <strong>Error:</strong> {{ run.error }}
        </div>
      </Card>

      <!-- Input/Output preview -->
      <Card title="Input / Output">
        <div class="io-section">
          <h4 class="sub-heading">User Message</h4>
          <div class="io-content">{{ run.input_preview || "(empty)" }}</div>
        </div>
        <div class="io-section">
          <h4 class="sub-heading">Assistant Response</h4>
          <div class="io-content" :class="{ 'error-text': run.error }">
            {{ run.output_preview || run.error || "(no response)" }}
          </div>
        </div>
      </Card>

      <!-- Node Trace -->
      <Card title="Execution Trace">
        <div v-if="run.nodes.length === 0" class="empty-msg dim">No nodes recorded</div>

        <div class="trace-list">
          <div
            v-for="node in run.nodes"
            :key="node.node_id"
            class="trace-node"
            :class="{
              selected: selectedNode?.node_id === node.node_id,
              'is-error': node.is_error,
            }"
            @click="selectNode(node)"
          >
            <div class="trace-icon">
              <span v-if="node.kind === 'llm_request'" class="kind-icon llm">LLM</span>
              <span v-else class="kind-icon tool">FN</span>
            </div>
            <div class="trace-body">
              <div class="trace-header">
                <span class="trace-name">{{ node.name }}</span>
                <RunStatusBadge :status="node.status" size="sm" />
                <span class="trace-dur mono">{{ formatDuration(node.duration_ms) }}</span>
              </div>
              <div class="trace-preview dim" v-if="node.kind === 'tool_call' && node.input_preview">
                {{ node.input_preview }}
              </div>
            </div>
            <div class="trace-tokens mono" v-if="node.kind === 'llm_request' && (node.input_tokens || node.output_tokens)">
              {{ formatTokens(node.input_tokens) }}/{{ formatTokens(node.output_tokens) }}
            </div>
          </div>
        </div>
      </Card>

      <!-- Node Inspector (expanded detail of selected node) -->
      <Card v-if="selectedNode" :title="`Node: ${selectedNode.name}`">
        <div class="inspector-grid">
          <div><span class="label">Node ID</span> <span class="mono val">#{{ selectedNode.node_id }}</span></div>
          <div><span class="label">Kind</span> <span class="val">{{ selectedNode.kind === 'llm_request' ? 'LLM Request' : 'Tool Call' }}</span></div>
          <div><span class="label">Status</span> <RunStatusBadge :status="selectedNode.status" size="sm" /></div>
          <div><span class="label">Duration</span> <span class="mono val">{{ formatDuration(selectedNode.duration_ms) }}</span></div>
          <div><span class="label">Started</span> <span class="mono val">{{ formatTime(selectedNode.started_at) }}</span></div>
          <div v-if="selectedNode.ended_at"><span class="label">Ended</span> <span class="mono val">{{ formatTime(selectedNode.ended_at) }}</span></div>
          <div v-if="selectedNode.kind === 'llm_request'">
            <span class="label">Tokens</span>
            <span class="mono val">{{ formatTokens(selectedNode.input_tokens) }} in / {{ formatTokens(selectedNode.output_tokens) }} out</span>
          </div>
        </div>

        <div v-if="selectedNode.input_preview" style="margin-top: 0.8rem">
          <CodeAccordion label="Input" :content="selectedNode.input_preview" />
        </div>
        <div v-if="selectedNode.output_preview" style="margin-top: 0.5rem">
          <CodeAccordion label="Output" :content="selectedNode.output_preview" />
        </div>
      </Card>

      <!-- Statistics -->
      <Card title="Statistics">
        <div class="stats-grid">
          <div class="stat">
            <span class="stat-num">{{ run.nodes.length }}</span>
            <span class="stat-label">nodes</span>
          </div>
          <div class="stat">
            <span class="stat-num">{{ llmNodes.length }}</span>
            <span class="stat-label">LLM calls</span>
          </div>
          <div class="stat">
            <span class="stat-num">{{ toolNodes.length }}</span>
            <span class="stat-label">tool calls</span>
          </div>
          <div class="stat">
            <span class="stat-num">{{ toolNodes.filter(n => n.is_error).length }}</span>
            <span class="stat-label">tool errors</span>
          </div>
          <div class="stat">
            <span class="stat-num">{{ run.loop_count }}</span>
            <span class="stat-label">loops</span>
          </div>
        </div>
      </Card>
    </template>
  </div>
</template>

<style scoped>
.header-bar {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  margin-bottom: 1.5rem;
}
.back-link { font-size: 1.2rem; color: var(--accent); text-decoration: none; }
.back-link:hover { text-decoration: underline; }
.sep { color: var(--text-dim); }
.run-id { font-size: 1rem; color: var(--text); }
.live-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: var(--green);
  animation: pulse-glow 1.5s ease-in-out infinite;
  margin-left: 0.3rem;
}
@keyframes pulse-glow {
  0%, 100% { opacity: 0.5; box-shadow: 0 0 4px var(--green); }
  50% { opacity: 1; box-shadow: 0 0 8px var(--green); }
}

.error { color: var(--red); margin-bottom: 1rem; }
.dim { color: var(--text-dim); font-size: 0.85rem; }
.mono { font-family: var(--mono); font-size: 0.82rem; }

/* Meta grid */
.meta-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 0.4rem 1.5rem;
  font-size: 0.85rem;
}
.label { color: var(--text-dim); margin-right: 0.5em; font-size: 0.78rem; }
.val { color: var(--text); }
.link { color: var(--accent); font-size: 0.82rem; }

.error-block {
  margin-top: 0.8rem;
  padding: 0.5rem 0.8rem;
  background: rgba(248, 81, 73, 0.08);
  border: 1px solid var(--red);
  border-radius: 4px;
  color: var(--red);
  font-size: 0.85rem;
}

/* I/O section */
.io-section { margin-bottom: 0.8rem; }
.sub-heading {
  color: var(--text-dim);
  font-size: 0.78rem;
  font-weight: 600;
  text-transform: uppercase;
  margin: 0 0 0.3rem;
  letter-spacing: 0.04em;
}
.io-content {
  padding: 0.5rem 0.8rem;
  background: var(--bg);
  border: 1px solid var(--border);
  border-radius: 4px;
  font-size: 0.85rem;
  white-space: pre-wrap;
  word-break: break-word;
  max-height: 200px;
  overflow-y: auto;
}
.error-text { color: var(--red); }

/* Trace list */
.trace-list {
  display: flex;
  flex-direction: column;
  gap: 0;
}
.trace-node {
  display: flex;
  align-items: flex-start;
  gap: 0.6rem;
  padding: 0.5rem 0.6rem;
  border-bottom: 1px solid var(--border);
  cursor: pointer;
  transition: background 0.1s;
}
.trace-node:hover { background: rgba(88, 166, 255, 0.03); }
.trace-node.selected { background: rgba(88, 166, 255, 0.08); border-left: 2px solid var(--accent); }
.trace-node.is-error { border-left: 2px solid var(--red); }

.trace-icon { flex-shrink: 0; padding-top: 0.1rem; }
.kind-icon {
  display: inline-block;
  padding: 0.1rem 0.4rem;
  border-radius: 3px;
  font-size: 0.65rem;
  font-weight: 700;
  font-family: var(--mono);
  letter-spacing: 0.03em;
}
.kind-icon.llm { background: rgba(88, 166, 255, 0.15); color: var(--accent); }
.kind-icon.tool { background: rgba(63, 185, 80, 0.15); color: var(--green); }

.trace-body { flex: 1; min-width: 0; }
.trace-header {
  display: flex;
  align-items: center;
  gap: 0.5rem;
}
.trace-name { font-weight: 600; font-size: 0.85rem; }
.trace-dur { margin-left: auto; color: var(--text-dim); }
.trace-preview {
  margin-top: 0.15rem;
  font-size: 0.78rem;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  max-width: 500px;
}

.trace-tokens {
  flex-shrink: 0;
  color: var(--text-dim);
  font-size: 0.75rem;
}

.empty-msg { padding: 1rem 0; text-align: center; }

/* Inspector */
.inspector-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 0.3rem 1.5rem;
  font-size: 0.85rem;
}

/* Statistics */
.stats-grid {
  display: flex;
  gap: 2rem;
}
.stat {
  display: flex;
  flex-direction: column;
  align-items: center;
}
.stat-num { font-size: 1.4rem; font-weight: 700; color: var(--text); }
.stat-label { font-size: 0.78rem; color: var(--text-dim); }
</style>
