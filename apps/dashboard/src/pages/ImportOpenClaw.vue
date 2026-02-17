<script setup lang="ts">
import { ref, computed } from "vue";
import { api } from "@/api/client";
import type { ScanResult, ImportApplyResult } from "@/api/client";
import Card from "@/components/Card.vue";
import StatusDot from "@/components/StatusDot.vue";

// ── Wizard steps ─────────────────────────────────────────────────
type Step = "path" | "review" | "options" | "result";
const step = ref<Step>("path");

// ── Step 1: path input ──────────────────────────────────────────
const scanPath = ref("/var/lib/serialagent/imports/openclaw");
const scanning = ref(false);
const scanError = ref("");
const scanResult = ref<ScanResult | null>(null);

async function doScan() {
  scanning.value = true;
  scanError.value = "";
  scanResult.value = null;
  try {
    scanResult.value = await api.scanOpenClaw(scanPath.value);
    step.value = "review";
  } catch (e: any) {
    scanError.value = e.message;
  } finally {
    scanning.value = false;
  }
}

// ── Step 2: review + select ─────────────────────────────────────
const selectedWorkspaces = ref<Set<string>>(new Set());
const selectedAgents = ref<Set<string>>(new Set());

function toggleWorkspace(name: string) {
  const s = selectedWorkspaces.value;
  if (s.has(name)) s.delete(name); else s.add(name);
}
function toggleAgent(name: string) {
  const s = selectedAgents.value;
  if (s.has(name)) s.delete(name); else s.add(name);
}
function selectAll() {
  scanResult.value?.workspaces.forEach(w => selectedWorkspaces.value.add(w.name));
  scanResult.value?.agents.forEach(a => selectedAgents.value.add(a.name));
}

function proceedToOptions() {
  step.value = "options";
}

// ── Step 3: import options ──────────────────────────────────────
const importModels = ref(true);
const importAuth = ref(false);
const importSessions = ref(false);
const applying = ref(false);
const applyError = ref("");
const applyResult = ref<ImportApplyResult | null>(null);

const hasAnyAuth = computed(() =>
  scanResult.value?.agents.some(a =>
    selectedAgents.value.has(a.name) && a.has_auth
  ) ?? false
);

async function doApply() {
  applying.value = true;
  applyError.value = "";
  applyResult.value = null;
  try {
    applyResult.value = await api.applyOpenClawImport({
      path: scanPath.value,
      workspaces: [...selectedWorkspaces.value],
      agents: [...selectedAgents.value],
      import_models: importModels.value,
      import_auth: importAuth.value,
      import_sessions: importSessions.value,
    });
    step.value = "result";
  } catch (e: any) {
    applyError.value = e.message;
  } finally {
    applying.value = false;
  }
}

function startOver() {
  step.value = "path";
  scanResult.value = null;
  applyResult.value = null;
  selectedWorkspaces.value = new Set();
  selectedAgents.value = new Set();
  importModels.value = true;
  importAuth.value = false;
  importSessions.value = false;
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}
</script>

<template>
  <div>
    <h1 class="page-title">Import OpenClaw</h1>

    <!-- Step indicator -->
    <div class="steps">
      <span :class="{ active: step === 'path' }">1. Scan</span>
      <span class="sep">&rarr;</span>
      <span :class="{ active: step === 'review' }">2. Review</span>
      <span class="sep">&rarr;</span>
      <span :class="{ active: step === 'options' }">3. Options</span>
      <span class="sep">&rarr;</span>
      <span :class="{ active: step === 'result' }">4. Done</span>
    </div>

    <!-- ── STEP 1: Path ────────────────────────────────────────── -->
    <Card v-if="step === 'path'" title="Scan OpenClaw Directory">
      <p class="hint">
        Point to a local directory containing OpenClaw data
        (e.g. <code>~/.openclaw</code> or an rsync'd import folder).
      </p>
      <div class="field">
        <label>Path</label>
        <input v-model="scanPath" placeholder="/var/lib/serialagent/imports/openclaw" />
      </div>
      <button @click="doScan" :disabled="scanning || !scanPath.trim()">
        {{ scanning ? "Scanning..." : "Scan" }}
      </button>
      <p v-if="scanError" class="error">{{ scanError }}</p>
    </Card>

    <!-- ── STEP 2: Review ──────────────────────────────────────── -->
    <template v-if="step === 'review' && scanResult">
      <Card v-if="!scanResult.valid" title="Nothing Found">
        <p class="dim">No importable agents or workspaces found at <code>{{ scanResult.path }}</code></p>
        <button @click="startOver">Back</button>
      </Card>

      <template v-else>
        <!-- Warnings -->
        <Card v-if="scanResult.warnings.length" title="Warnings">
          <div v-for="w in scanResult.warnings" :key="w" class="warning-row">
            <StatusDot status="warn" /> {{ w }}
          </div>
        </Card>

        <!-- Workspaces -->
        <Card title="Workspaces">
          <table class="tbl" v-if="scanResult.workspaces.length">
            <thead>
              <tr><th></th><th>Name</th><th>Files</th><th>Size</th></tr>
            </thead>
            <tbody>
              <tr
                v-for="ws in scanResult.workspaces" :key="ws.name"
                class="clickable"
                @click="toggleWorkspace(ws.name)"
              >
                <td><input type="checkbox" :checked="selectedWorkspaces.has(ws.name)" /></td>
                <td><code>{{ ws.name }}</code></td>
                <td>{{ ws.files.length }} files</td>
                <td class="dim">{{ formatBytes(ws.total_size_bytes) }}</td>
              </tr>
            </tbody>
          </table>
          <p v-else class="dim">No workspaces found</p>
        </Card>

        <!-- Agents -->
        <Card title="Agents">
          <table class="tbl" v-if="scanResult.agents.length">
            <thead>
              <tr><th></th><th>Name</th><th>Models</th><th>Auth</th><th>Sessions</th></tr>
            </thead>
            <tbody>
              <tr
                v-for="a in scanResult.agents" :key="a.name"
                class="clickable"
                @click="toggleAgent(a.name)"
              >
                <td><input type="checkbox" :checked="selectedAgents.has(a.name)" /></td>
                <td><code>{{ a.name }}</code></td>
                <td>
                  <StatusDot :status="a.has_models ? 'ok' : 'off'" />
                  {{ a.has_models ? "Yes" : "No" }}
                </td>
                <td>
                  <StatusDot :status="a.has_auth ? 'warn' : 'off'" />
                  {{ a.has_auth ? "Yes" : "No" }}
                </td>
                <td class="dim">{{ a.session_count }} JSONL</td>
              </tr>
            </tbody>
          </table>
          <p v-else class="dim">No agents found</p>
        </Card>

        <div class="action-bar">
          <button class="secondary" @click="selectAll">Select All</button>
          <button @click="proceedToOptions" :disabled="selectedWorkspaces.size === 0 && selectedAgents.size === 0">
            Next: Import Options
          </button>
          <button class="secondary" @click="startOver">Back</button>
        </div>
      </template>
    </template>

    <!-- ── STEP 3: Options ─────────────────────────────────────── -->
    <Card v-if="step === 'options'" title="Import Options">
      <div class="summary">
        <p><strong>{{ selectedWorkspaces.size }}</strong> workspace{{ selectedWorkspaces.size !== 1 ? "s" : "" }} selected</p>
        <p><strong>{{ selectedAgents.size }}</strong> agent{{ selectedAgents.size !== 1 ? "s" : "" }} selected</p>
      </div>

      <div class="option-list">
        <label class="option">
          <input type="checkbox" checked disabled />
          <span>Import workspace files</span>
          <span class="dim"> (always included)</span>
        </label>

        <label v-if="selectedAgents.size > 0" class="option">
          <input type="checkbox" v-model="importModels" />
          <span>Import models.json</span>
          <span class="dim"> (model choices per agent)</span>
        </label>

        <label v-if="selectedAgents.size > 0 && hasAnyAuth" class="option warning-option">
          <input type="checkbox" v-model="importAuth" />
          <span>Import auth-profiles.json</span>
          <span class="dim warn-text"> (contains API keys — handle with care)</span>
        </label>

        <label v-if="selectedAgents.size > 0" class="option">
          <input type="checkbox" v-model="importSessions" />
          <span>Import session transcripts (JSONL)</span>
          <span class="dim"> (may be large)</span>
        </label>
      </div>

      <div v-if="importAuth" class="auth-warning">
        <StatusDot status="warn" />
        <strong>Credential import enabled.</strong>
        auth-profiles.json contains plaintext API keys. Ensure these are rotated if the source machine is shared.
      </div>

      <div class="action-bar">
        <button @click="doApply" :disabled="applying">
          {{ applying ? "Importing..." : "Apply Import" }}
        </button>
        <button class="secondary" @click="step = 'review'">Back</button>
      </div>
      <p v-if="applyError" class="error">{{ applyError }}</p>
    </Card>

    <!-- ── STEP 4: Result ──────────────────────────────────────── -->
    <template v-if="step === 'result' && applyResult">
      <Card :title="applyResult.success ? 'Import Complete' : 'Import Completed with Errors'">
        <p>
          <StatusDot :status="applyResult.success ? 'ok' : 'warn'" />
          <strong>{{ applyResult.files_copied }}</strong> files copied
        </p>

        <div class="result-grid">
          <div v-if="applyResult.workspaces_imported.length">
            <span class="label">Workspaces</span>
            <code v-for="w in applyResult.workspaces_imported" :key="w" class="tag">{{ w }}</code>
          </div>
          <div v-if="applyResult.agents_imported.length">
            <span class="label">Agents</span>
            <code v-for="a in applyResult.agents_imported" :key="a" class="tag">{{ a }}</code>
          </div>
          <div v-if="applyResult.sessions_imported > 0">
            <span class="label">Sessions</span>
            {{ applyResult.sessions_imported }} imported
          </div>
        </div>

        <div v-if="applyResult.warnings.length" class="result-section">
          <h4 class="sub-heading">Warnings</h4>
          <p v-for="w in applyResult.warnings" :key="w" class="warn-line">
            <StatusDot status="warn" /> {{ w }}
          </p>
        </div>

        <div v-if="applyResult.errors.length" class="result-section">
          <h4 class="sub-heading">Errors</h4>
          <p v-for="e in applyResult.errors" :key="e" class="error-line">
            <StatusDot status="error" /> {{ e }}
          </p>
        </div>

        <div class="action-bar">
          <button @click="startOver">Import Another</button>
          <router-link to="/" class="secondary-link">Back to Overview</router-link>
        </div>
      </Card>
    </template>
  </div>
</template>

<style scoped>
.page-title { font-size: 1.5rem; margin-bottom: 1.5rem; color: var(--accent); }
.error { color: var(--red); margin-top: 0.6rem; }
.dim { color: var(--text-dim); font-size: 0.85rem; }
.hint { color: var(--text-dim); font-size: 0.88rem; margin-bottom: 0.8rem; }

.steps {
  display: flex;
  gap: 0.5rem;
  align-items: center;
  margin-bottom: 1.5rem;
  font-size: 0.85rem;
  color: var(--text-dim);
}
.steps .active { color: var(--accent); font-weight: 600; }
.steps .sep { color: var(--border); }

.field { display: flex; flex-direction: column; gap: 0.2rem; margin-bottom: 0.8rem; }
.field label { font-size: 0.78rem; color: var(--text-dim); }
.field input {
  background: var(--bg);
  border: 1px solid var(--border);
  color: var(--text);
  padding: 0.5rem 0.8rem;
  border-radius: 4px;
  font-family: var(--mono);
  font-size: 0.88rem;
  width: 100%;
}

button {
  background: var(--accent-dim);
  color: white;
  border: none;
  padding: 0.5rem 1.2rem;
  border-radius: 4px;
  cursor: pointer;
  font-size: 0.85rem;
}
button:disabled { opacity: 0.5; cursor: not-allowed; }
button:hover:not(:disabled) { background: var(--accent); }
button.secondary {
  background: transparent;
  border: 1px solid var(--border);
  color: var(--text-dim);
}
button.secondary:hover { color: var(--text); border-color: var(--text-dim); }

.action-bar { display: flex; gap: 0.6rem; align-items: center; margin-top: 1rem; }
.secondary-link { font-size: 0.85rem; }

.tbl { width: 100%; border-collapse: collapse; font-size: 0.85rem; }
.tbl th { color: var(--text-dim); font-weight: 600; text-align: left; padding: 0.5rem 0.6rem; border-bottom: 1px solid var(--border); }
.tbl td { padding: 0.5rem 0.6rem; border-bottom: 1px solid var(--border); }
.clickable { cursor: pointer; }
.clickable:hover { background: rgba(88, 166, 255, 0.05); }

.warning-row { font-size: 0.85rem; padding: 0.3rem 0; color: var(--yellow); }

.summary { margin-bottom: 1rem; font-size: 0.9rem; }
.summary p { margin: 0.2rem 0; }

.option-list { display: flex; flex-direction: column; gap: 0.5rem; }
.option {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  font-size: 0.88rem;
  cursor: pointer;
}
.option input { cursor: pointer; }
.warning-option { border-left: 2px solid var(--yellow); padding-left: 0.5rem; }
.warn-text { color: var(--yellow); }

.auth-warning {
  background: rgba(210, 153, 34, 0.1);
  border: 1px solid var(--yellow);
  border-radius: 4px;
  padding: 0.6rem 0.8rem;
  margin-top: 0.8rem;
  font-size: 0.85rem;
}

.result-grid {
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
  margin-top: 0.8rem;
  font-size: 0.88rem;
}
.label { color: var(--text-dim); margin-right: 0.5em; }
.tag {
  display: inline-block;
  background: #21262d;
  padding: 0.15rem 0.5rem;
  border-radius: 3px;
  font-size: 0.8rem;
  margin: 0 0.2rem;
}

.result-section { margin-top: 1rem; }
.sub-heading { color: var(--text-dim); font-size: 0.8rem; font-weight: 600; text-transform: uppercase; margin-bottom: 0.4rem; }
.warn-line { color: var(--yellow); font-size: 0.85rem; margin: 0.2rem 0; }
.error-line { color: var(--red); font-size: 0.85rem; margin: 0.2rem 0; }
</style>
