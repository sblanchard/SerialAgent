<script setup lang="ts">
import { ref, onMounted } from "vue";
import { api } from "@/api/client";
import type { ReadinessResponse, ProviderInfo, InitError } from "@/api/client";
import Card from "@/components/Card.vue";
import StatusDot from "@/components/StatusDot.vue";

const data = ref<ReadinessResponse | null>(null);
const error = ref("");

async function load() {
  try {
    data.value = await api.readiness();
  } catch (e: any) {
    error.value = e.message;
  }
}

function toolSupport(s: string): string {
  if (s.includes("StrictJson")) return "Strict JSON";
  if (s.includes("Basic")) return "Basic";
  return "None";
}

function toolSupportStatus(s: string): "ok" | "warn" | "off" {
  if (s.includes("StrictJson")) return "ok";
  if (s.includes("Basic")) return "warn";
  return "off";
}

onMounted(load);
</script>

<template>
  <div>
    <h1 class="page-title">LLM Readiness</h1>
    <p v-if="error" class="error">{{ error }}</p>

    <template v-if="data">
      <!-- Summary cards -->
      <div class="grid">
        <Card title="Status">
          <p>
            <StatusDot :status="data.ready ? 'ok' : 'error'" />
            <strong>{{ data.ready ? "Ready" : "Not Ready" }}</strong>
          </p>
          <p class="dim">Startup policy: <code>{{ data.startup_policy }}</code></p>
        </Card>

        <Card title="Providers">
          <p class="big-number">{{ data.provider_count }}</p>
          <p class="dim">initialized</p>
        </Card>

        <Card title="Executor">
          <p>
            <StatusDot :status="data.has_executor ? 'ok' : 'error'" />
            {{ data.has_executor ? "Available" : "Missing" }}
          </p>
          <p class="dim">Required for /v1/chat</p>
        </Card>

        <Card title="Connected Nodes">
          <p class="big-number">{{ data.nodes_connected }}</p>
          <p class="dim">node{{ data.nodes_connected !== 1 ? "s" : "" }}</p>
        </Card>
      </div>

      <!-- Role assignments -->
      <Card title="Role Assignments">
        <table class="tbl" v-if="Object.keys(data.roles).length > 0">
          <thead>
            <tr><th>Role</th><th>Model</th></tr>
          </thead>
          <tbody>
            <tr v-for="(model, role) in data.roles" :key="role">
              <td><code>{{ role }}</code></td>
              <td><code>{{ model }}</code></td>
            </tr>
          </tbody>
        </table>
        <p v-else class="dim" style="text-align:center;padding:1rem">No role assignments</p>
      </Card>

      <!-- Provider details -->
      <Card title="Provider Capabilities">
        <table class="tbl" v-if="data.providers.length > 0">
          <thead>
            <tr>
              <th>Provider</th>
              <th>Tools</th>
              <th>Streaming</th>
              <th>JSON Mode</th>
              <th>Vision</th>
              <th>Context Window</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="p in data.providers" :key="p.id">
              <td><code>{{ p.id }}</code></td>
              <td>
                <StatusDot :status="toolSupportStatus(p.capabilities.supports_tools)" />
                {{ toolSupport(p.capabilities.supports_tools) }}
              </td>
              <td>
                <StatusDot :status="p.capabilities.supports_streaming ? 'ok' : 'off'" />
                {{ p.capabilities.supports_streaming ? "Yes" : "No" }}
              </td>
              <td>
                <StatusDot :status="p.capabilities.supports_json_mode ? 'ok' : 'off'" />
                {{ p.capabilities.supports_json_mode ? "Yes" : "No" }}
              </td>
              <td>
                <StatusDot :status="p.capabilities.supports_vision ? 'ok' : 'off'" />
                {{ p.capabilities.supports_vision ? "Yes" : "No" }}
              </td>
              <td class="dim">{{ p.capabilities.context_window_tokens?.toLocaleString() ?? "-" }}</td>
            </tr>
          </tbody>
        </table>
        <p v-else class="dim" style="text-align:center;padding:1rem">No providers initialized</p>
      </Card>

      <!-- Init errors -->
      <Card v-if="data.init_errors.length > 0" title="Initialization Errors">
        <div v-for="e in data.init_errors" :key="e.provider_id" class="init-error">
          <p>
            <StatusDot status="error" />
            <strong>{{ e.provider_id }}</strong>
            <span class="dim"> ({{ e.kind }})</span>
          </p>
          <pre class="error-detail">{{ e.error }}</pre>
        </div>
      </Card>
    </template>
  </div>
</template>

<style scoped>
.page-title { font-size: 1.5rem; margin-bottom: 1.5rem; color: var(--accent); }
.grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(200px, 1fr)); gap: 1rem; margin-bottom: 1.5rem; }
.big-number { font-size: 2.2rem; font-weight: 700; color: var(--text); }
.dim { color: var(--text-dim); font-size: 0.85rem; margin-top: 0.3rem; }
.error { color: var(--red); margin-bottom: 1rem; }
.tbl { width: 100%; border-collapse: collapse; font-size: 0.85rem; }
.tbl th { color: var(--text-dim); font-weight: 600; text-align: left; padding: 0.5rem 0.6rem; border-bottom: 1px solid var(--border); }
.tbl td { padding: 0.5rem 0.6rem; border-bottom: 1px solid var(--border); }
.init-error { margin-bottom: 0.8rem; }
.error-detail {
  background: var(--bg);
  border: 1px solid var(--border);
  padding: 0.5rem;
  border-radius: 4px;
  font-family: var(--mono);
  font-size: 0.8rem;
  color: var(--red);
  margin-top: 0.3rem;
  white-space: pre-wrap;
}
</style>
