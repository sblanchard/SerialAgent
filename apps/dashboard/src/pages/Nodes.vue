<script setup lang="ts">
import { ref, onMounted, computed } from "vue";
import { api } from "@/api/client";
import type { NodeInfo, ToolInvokeResponse } from "@/api/client";
import Card from "@/components/Card.vue";
import StatusDot from "@/components/StatusDot.vue";

const nodes = ref<NodeInfo[]>([]);
const error = ref("");
const selected = ref<NodeInfo | null>(null);

// Tool Ping panel state
const pingTool = ref("");
const pingArgs = ref("{}");
const pingResult = ref<ToolInvokeResponse | null>(null);
const pinging = ref(false);
const pingError = ref("");

const allCapabilities = computed(() => {
  const caps: string[] = [];
  for (const n of nodes.value) {
    for (const c of n.capabilities) {
      if (!caps.includes(c)) caps.push(c);
    }
  }
  return caps.sort();
});

async function load() {
  try {
    const res = await api.nodes();
    nodes.value = res.nodes;
  } catch (e: any) {
    error.value = e.message;
  }
}

function selectNode(n: NodeInfo) {
  selected.value = selected.value?.node_id === n.node_id ? null : n;
  // Pre-fill first capability as tool name
  if (selected.value && selected.value.capabilities.length) {
    pingTool.value = selected.value.capabilities[0];
  }
}

async function doPing() {
  pinging.value = true;
  pingError.value = "";
  pingResult.value = null;
  try {
    const args = JSON.parse(pingArgs.value);
    pingResult.value = await api.invokeTool({
      tool: pingTool.value,
      args,
      timeout_ms: 10000,
    });
  } catch (e: any) {
    pingError.value = e.message;
  } finally {
    pinging.value = false;
  }
}

function timeSince(iso: string): string {
  const ms = Date.now() - new Date(iso).getTime();
  const sec = Math.floor(ms / 1000);
  if (sec < 60) return `${sec}s ago`;
  const min = Math.floor(sec / 60);
  if (min < 60) return `${min}m ago`;
  return `${Math.floor(min / 60)}h ago`;
}

onMounted(load);
</script>

<template>
  <div>
    <h1 class="page-title">Nodes</h1>
    <p v-if="error" class="error">{{ error }}</p>

    <Card>
      <table class="tbl">
        <thead>
          <tr>
            <th></th>
            <th>Node ID</th>
            <th>Type</th>
            <th>Name</th>
            <th>Capabilities</th>
            <th>Version</th>
            <th>Last Seen</th>
          </tr>
        </thead>
        <tbody>
          <tr
            v-for="n in nodes"
            :key="n.node_id"
            class="clickable"
            :class="{ 'selected-row': selected?.node_id === n.node_id }"
            @click="selectNode(n)"
          >
            <td><StatusDot status="ok" /></td>
            <td><code>{{ n.node_id }}</code></td>
            <td>{{ n.node_type }}</td>
            <td>{{ n.name }}</td>
            <td>{{ n.capabilities.length }}</td>
            <td class="dim"><code>{{ n.version }}</code></td>
            <td class="dim">{{ timeSince(n.last_seen) }}</td>
          </tr>
          <tr v-if="nodes.length === 0">
            <td colspan="7" class="dim" style="text-align:center;padding:2rem">No nodes connected</td>
          </tr>
        </tbody>
      </table>
    </Card>

    <!-- Node detail drawer -->
    <div v-if="selected" class="drawer">
      <Card :title="`Node: ${selected.node_id}`">
        <div class="detail-grid">
          <div><span class="label">ID</span><code>{{ selected.node_id }}</code></div>
          <div><span class="label">Type</span>{{ selected.node_type }}</div>
          <div><span class="label">Name</span>{{ selected.name }}</div>
          <div><span class="label">Version</span><code>{{ selected.version }}</code></div>
          <div><span class="label">Session</span><code class="dim">{{ selected.session_id }}</code></div>
          <div><span class="label">Connected</span><span class="dim">{{ new Date(selected.connected_at).toLocaleString() }}</span></div>
          <div><span class="label">Last Seen</span><span class="dim">{{ timeSince(selected.last_seen) }}</span></div>
          <div v-if="selected.tags.length"><span class="label">Tags</span>{{ selected.tags.join(", ") }}</div>
        </div>

        <h4 class="sub-heading">Capabilities</h4>
        <ul class="cap-list">
          <li v-for="c in selected.capabilities" :key="c">
            <code>{{ c }}</code>
          </li>
        </ul>
      </Card>

      <!-- Tool Ping -->
      <Card title="Tool Ping">
        <div class="ping-form">
          <div class="field">
            <label>Tool name</label>
            <input v-model="pingTool" list="cap-autocomplete" placeholder="macos.notes.search" />
            <datalist id="cap-autocomplete">
              <option v-for="c in allCapabilities" :key="c" :value="c" />
            </datalist>
          </div>
          <div class="field">
            <label>Args (JSON)</label>
            <textarea v-model="pingArgs" rows="3" spellcheck="false" />
          </div>
          <button @click="doPing" :disabled="pinging || !pingTool">
            {{ pinging ? "Sending..." : "Invoke" }}
          </button>
        </div>

        <p v-if="pingError" class="error">{{ pingError }}</p>

        <div v-if="pingResult" class="ping-result">
          <p>
            <StatusDot :status="pingResult.ok ? 'ok' : 'error'" />
            {{ pingResult.ok ? "Success" : "Error" }}
            <span class="dim"> &mdash; {{ pingResult.duration_ms }}ms via {{ pingResult.route.kind }}{{ pingResult.route.node_id ? `:${pingResult.route.node_id}` : '' }}</span>
          </p>
          <pre class="json">{{ JSON.stringify(pingResult.ok ? pingResult.result : pingResult.error, null, 2) }}</pre>
        </div>
      </Card>
    </div>
  </div>
</template>

<style scoped>
.page-title { font-size: 1.5rem; margin-bottom: 1.5rem; color: var(--accent); }
.error { color: var(--red); margin-bottom: 1rem; }
.tbl { width: 100%; border-collapse: collapse; font-size: 0.85rem; }
.tbl th { color: var(--text-dim); font-weight: 600; text-align: left; padding: 0.5rem 0.6rem; border-bottom: 1px solid var(--border); }
.tbl td { padding: 0.5rem 0.6rem; border-bottom: 1px solid var(--border); }
.dim { color: var(--text-dim); }
.clickable { cursor: pointer; }
.clickable:hover { background: rgba(88, 166, 255, 0.05); }
.selected-row { background: rgba(88, 166, 255, 0.1) !important; }
.drawer { margin-top: 1rem; }
.detail-grid { display: grid; grid-template-columns: 1fr 1fr; gap: 0.5rem 2rem; font-size: 0.88rem; }
.label { color: var(--text-dim); margin-right: 0.5em; }
.sub-heading { color: var(--text-dim); font-size: 0.8rem; font-weight: 600; text-transform: uppercase; margin: 1rem 0 0.5rem; letter-spacing: 0.05em; }
.cap-list { list-style: none; display: flex; flex-wrap: wrap; gap: 0.4rem; }
.cap-list li { background: #21262d; padding: 0.2rem 0.6rem; border-radius: 4px; font-size: 0.82rem; }
.ping-form { display: flex; flex-direction: column; gap: 0.6rem; }
.field { display: flex; flex-direction: column; gap: 0.2rem; }
.field label { font-size: 0.78rem; color: var(--text-dim); }
.field input, .field textarea {
  background: var(--bg);
  border: 1px solid var(--border);
  color: var(--text);
  padding: 0.4rem 0.6rem;
  border-radius: 4px;
  font-family: var(--mono);
  font-size: 0.85rem;
}
button {
  background: var(--accent-dim);
  color: white;
  border: none;
  padding: 0.5rem 1.2rem;
  border-radius: 4px;
  cursor: pointer;
  font-size: 0.85rem;
  align-self: flex-start;
}
button:disabled { opacity: 0.5; cursor: not-allowed; }
button:hover:not(:disabled) { background: var(--accent); }
.ping-result { margin-top: 0.8rem; }
.json {
  background: var(--bg);
  border: 1px solid var(--border);
  padding: 0.6rem;
  border-radius: 4px;
  font-family: var(--mono);
  font-size: 0.82rem;
  overflow-x: auto;
  max-height: 300px;
  overflow-y: auto;
  margin-top: 0.4rem;
  white-space: pre-wrap;
}
</style>
