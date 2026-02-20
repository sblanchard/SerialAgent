<script setup lang="ts">
import { ref, computed, onMounted } from "vue";
import { api, ApiError } from "@/api/client";
import type { SystemInfo, ReadinessResponse } from "@/api/client";
import Card from "@/components/Card.vue";
import LoadingPanel from "@/components/LoadingPanel.vue";
import ConfigEditor from "@/components/ConfigEditor.vue";
import { configToToml } from "@/utils/toml";

const sysInfo = ref<SystemInfo | null>(null);
const readiness = ref<ReadinessResponse | null>(null);
const loading = ref(true);
const error = ref("");
const editorMode = ref<"view" | "edit">("view");

const generatedToml = computed(() => {
  if (!sysInfo.value || !readiness.value) return "";
  return configToToml(sysInfo.value, readiness.value);
});

function toggleMode() {
  editorMode.value = editorMode.value === "view" ? "edit" : "view";
}

async function load() {
  loading.value = true;
  error.value = "";
  try {
    const [info, ready] = await Promise.all([
      api.systemInfo(),
      api.readiness(),
    ]);
    sysInfo.value = info;
    readiness.value = ready;
  } catch (e: unknown) {
    error.value = e instanceof ApiError ? e.friendly : String(e);
  } finally {
    loading.value = false;
  }
}

onMounted(load);
</script>

<template>
  <div>
    <div class="settings-header">
      <h1 class="page-title">Settings</h1>
      <button v-if="!loading" class="secondary" @click="toggleMode">
        {{ editorMode === "view" ? "Edit Config" : "View Info" }}
      </button>
    </div>

    <p v-if="error" class="error">{{ error }}</p>

    <Card v-if="loading" title="Loading">
      <LoadingPanel message="Loading system info..." />
    </Card>

    <!-- View mode: existing read-only cards -->
    <template v-if="!loading && editorMode === 'view'">
      <template v-if="sysInfo">
        <Card title="System">
          <div class="settings-grid">
            <div><span class="label">Version</span> <span class="mono val">{{ sysInfo.version }}</span></div>
            <div><span class="label">Host</span> <span class="mono val">{{ sysInfo.server.host }}:{{ sysInfo.server.port }}</span></div>
            <div><span class="label">Workspace</span> <span class="mono val">{{ sysInfo.workspace_path }}</span></div>
            <div><span class="label">Skills Path</span> <span class="mono val">{{ sysInfo.skills_path }}</span></div>
            <div><span class="label">SerialMemory</span> <span class="mono val">{{ sysInfo.serial_memory_url }}</span></div>
            <div><span class="label">Transport</span> <span class="mono val">{{ sysInfo.serial_memory_transport }}</span></div>
            <div><span class="label">Admin Token</span> <span class="val">{{ sysInfo.admin_token_set ? "Set" : "Not set" }}</span></div>
          </div>
        </Card>

        <Card title="Status">
          <div class="settings-grid">
            <div><span class="label">Providers</span> <span class="mono val">{{ sysInfo.provider_count }}</span></div>
            <div><span class="label">Nodes</span> <span class="mono val">{{ sysInfo.node_count }}</span></div>
            <div><span class="label">Sessions</span> <span class="mono val">{{ sysInfo.session_count }}</span></div>
          </div>
        </Card>
      </template>

      <template v-if="readiness">
        <Card title="LLM Providers">
          <div class="readiness-header">
            <span :class="readiness.ready ? 'status-ok' : 'status-warn'">
              {{ readiness.ready ? "Ready" : "Not Ready" }}
            </span>
            <span class="dim">{{ readiness.provider_count }} providers</span>
          </div>

          <div v-if="readiness.providers.length > 0" class="provider-list">
            <div v-for="p in readiness.providers" :key="p.id" class="provider-item">
              <span class="provider-id mono">{{ p.id }}</span>
              <span class="dim">
                ctx: {{ p.capabilities.context_window_tokens.toLocaleString() }} |
                tools: {{ p.capabilities.supports_tools }} |
                stream: {{ p.capabilities.supports_streaming ? "yes" : "no" }}
              </span>
            </div>
          </div>

          <div v-if="readiness.init_errors.length > 0" class="error-section">
            <div class="sub-heading">Init Errors</div>
            <div v-for="err in readiness.init_errors" :key="err.provider_id" class="error-item">
              <span class="mono">{{ err.provider_id }}</span>: {{ err.error }}
            </div>
          </div>

          <div v-if="Object.keys(readiness.roles).length > 0" class="roles-section">
            <div class="sub-heading">Role Assignments</div>
            <div v-for="(provider, role) in readiness.roles" :key="role" class="role-item">
              <span class="label">{{ role }}</span>
              <span class="mono val">{{ provider }}</span>
            </div>
          </div>
        </Card>

        <Card title="Environment">
          <div class="settings-grid">
            <div><span class="label">Has Executor</span> <span :class="readiness.has_executor ? 'status-ok' : 'status-warn'">{{ readiness.has_executor ? "Yes" : "No" }}</span></div>
            <div><span class="label">Memory Configured</span> <span :class="readiness.memory_configured ? 'status-ok' : 'status-warn'">{{ readiness.memory_configured ? "Yes" : "No" }}</span></div>
            <div><span class="label">Nodes Connected</span> <span class="mono val">{{ readiness.nodes_connected }}</span></div>
            <div><span class="label">Startup Policy</span> <span class="mono val">{{ readiness.startup_policy }}</span></div>
          </div>
        </Card>
      </template>
    </template>

    <!-- Edit mode: TOML config editor -->
    <Card v-if="!loading && editorMode === 'edit'" title="Configuration Editor">
      <ConfigEditor :initial-toml="generatedToml" />
    </Card>
  </div>
</template>

<style scoped>
.settings-header {
  display: flex;
  align-items: center;
  gap: 0.8rem;
  margin-bottom: 1.5rem;
}
.page-title { font-size: 1.5rem; color: var(--accent); margin: 0; }
.error { color: var(--red); margin-bottom: 1rem; }
.dim { color: var(--text-dim); font-size: 0.85rem; }
.mono { font-family: var(--mono); font-size: 0.82rem; }

.settings-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 0.4rem 1.5rem;
  font-size: 0.85rem;
}
.label { color: var(--text-dim); margin-right: 0.5em; font-size: 0.78rem; }
.val { color: var(--text); }

.readiness-header {
  display: flex;
  align-items: center;
  gap: 1rem;
  margin-bottom: 0.8rem;
}
.status-ok { color: var(--green); font-weight: 600; font-size: 0.85rem; }
.status-warn { color: var(--red); font-weight: 600; font-size: 0.85rem; }

.provider-list { margin-bottom: 0.8rem; }
.provider-item {
  display: flex;
  align-items: center;
  gap: 0.8rem;
  padding: 0.3rem 0;
  border-bottom: 1px solid var(--border);
  font-size: 0.85rem;
}
.provider-id { font-weight: 600; }

.error-section, .roles-section { margin-top: 0.8rem; }
.sub-heading {
  color: var(--text-dim);
  font-size: 0.78rem;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.04em;
  margin-bottom: 0.3rem;
}
.error-item {
  color: var(--red);
  font-size: 0.82rem;
  padding: 0.2rem 0;
}
.role-item {
  display: flex;
  gap: 0.5rem;
  font-size: 0.85rem;
  padding: 0.2rem 0;
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
button.secondary:hover { color: var(--text); border-color: var(--text-dim); }
</style>
