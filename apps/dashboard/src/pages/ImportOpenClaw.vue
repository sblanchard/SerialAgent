<script setup lang="ts">
import { ref, computed } from "vue";
import { api } from "@/api/client";
import type {
  ImportSource,
  ImportOptions,
  ImportPreviewResponse,
  ImportApplyResponseV2,
  MergeStrategy,
  SshAuth,
} from "@/api/client";
import Card from "@/components/Card.vue";
import StatusDot from "@/components/StatusDot.vue";

// ── Wizard steps ─────────────────────────────────────────────────
type Step = "source" | "preview" | "apply" | "result";
const step = ref<Step>("source");

// ── Step 0: Source selection ─────────────────────────────────────
type SourceTab = "local" | "ssh";
const sourceTab = ref<SourceTab>("local");

// Local source
const localPath = ref("/home/user/.openclaw");

// SSH source
const sshHost = ref("");
const sshUser = ref("");
const sshPort = ref("");
const sshRemotePath = ref("~/.openclaw");
const sshAuthMethod = ref<"agent" | "keyfile">("agent");
const sshKeyPath = ref("~/.ssh/id_ed25519");
const sshTesting = ref(false);
const sshTestResult = ref<{ ok: boolean; message: string } | null>(null);

// Presets
type Preset = "minimal" | "full" | "everything" | "custom";
const preset = ref<Preset>("full");

const options = computed<ImportOptions>(() => {
  switch (preset.value) {
    case "minimal":
      return { include_workspaces: true, include_sessions: true };
    case "full":
      return { include_workspaces: true, include_sessions: true, include_models: true };
    case "everything":
      return {
        include_workspaces: true,
        include_sessions: true,
        include_models: true,
        include_auth_profiles: true,
      };
    case "custom":
      return {
        include_workspaces: customWorkspaces.value,
        include_sessions: customSessions.value,
        include_models: customModels.value,
        include_auth_profiles: customAuth.value,
      };
  }
});

// Custom options
const customWorkspaces = ref(true);
const customSessions = ref(true);
const customModels = ref(false);
const customAuth = ref(false);

// Scan state
const scanning = ref(false);
const scanError = ref("");
const previewData = ref<ImportPreviewResponse | null>(null);

function buildSource(): ImportSource {
  if (sourceTab.value === "local") {
    return { local: { path: localPath.value } };
  }
  const auth: SshAuth = sshAuthMethod.value === "keyfile"
    ? { key_file: { key_path: sshKeyPath.value } }
    : "agent";
  return {
    ssh: {
      host: sshHost.value,
      user: sshUser.value || undefined,
      port: sshPort.value ? parseInt(sshPort.value) : undefined,
      remote_path: sshRemotePath.value,
      auth,
    },
  };
}

async function testSsh() {
  sshTesting.value = true;
  sshTestResult.value = null;
  try {
    const res = await api.testSsh(
      sshHost.value,
      sshUser.value || undefined,
      sshPort.value ? parseInt(sshPort.value) : undefined
    );
    sshTestResult.value = {
      ok: res.ok,
      message: res.ok ? "Connection successful" : (res.stderr || res.error || "Failed"),
    };
  } catch (e: any) {
    sshTestResult.value = { ok: false, message: e.message };
  } finally {
    sshTesting.value = false;
  }
}

async function doPreview() {
  scanning.value = true;
  scanError.value = "";
  previewData.value = null;
  try {
    previewData.value = await api.importPreview({
      source: buildSource(),
      options: options.value,
    });
    step.value = "preview";
  } catch (e: any) {
    scanError.value = e.message;
  } finally {
    scanning.value = false;
  }
}

const canScan = computed(() => {
  if (scanning.value) return false;
  if (sourceTab.value === "local") return !!localPath.value.trim();
  return !!sshHost.value.trim();
});

// ── Step 1: Preview ─────────────────────────────────────────────
const mergeStrategy = ref<MergeStrategy>("merge_safe");

function proceedToApply() {
  step.value = "apply";
}

// ── Step 2: Apply ───────────────────────────────────────────────
const applying = ref(false);
const applyError = ref("");
const applyResult = ref<ImportApplyResponseV2 | null>(null);

async function doApply() {
  if (!previewData.value) return;
  applying.value = true;
  applyError.value = "";
  try {
    applyResult.value = await api.importApply({
      staging_id: previewData.value.staging_id,
      merge_strategy: mergeStrategy.value,
      options: options.value,
    });
    step.value = "result";
  } catch (e: any) {
    applyError.value = e.message;
  } finally {
    applying.value = false;
  }
}

// ── Helpers ─────────────────────────────────────────────────────
function startOver() {
  step.value = "source";
  previewData.value = null;
  applyResult.value = null;
  scanError.value = "";
  applyError.value = "";
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

const strategyLabel: Record<MergeStrategy, string> = {
  merge_safe: "Safe Merge (imported/openclaw/...)",
  replace: "Replace Existing",
  skip_existing: "Skip Existing Files",
};
</script>

<template>
  <div>
    <h1 class="page-title">Import OpenClaw</h1>

    <!-- Step indicator -->
    <div class="steps">
      <span :class="{ active: step === 'source' }">1. Source</span>
      <span class="sep">&rarr;</span>
      <span :class="{ active: step === 'preview' }">2. Preview</span>
      <span class="sep">&rarr;</span>
      <span :class="{ active: step === 'apply' }">3. Apply</span>
      <span class="sep">&rarr;</span>
      <span :class="{ active: step === 'result' }">4. Done</span>
    </div>

    <!-- ── STEP 1: Source ──────────────────────────────────────── -->
    <template v-if="step === 'source'">
      <Card title="Import Source">
        <div class="tab-bar">
          <button
            :class="{ 'tab-active': sourceTab === 'local' }"
            @click="sourceTab = 'local'"
          >Local Directory</button>
          <button
            :class="{ 'tab-active': sourceTab === 'ssh' }"
            @click="sourceTab = 'ssh'"
          >Remote (SSH)</button>
        </div>

        <!-- Local tab -->
        <div v-if="sourceTab === 'local'" class="tab-content">
          <div class="field">
            <label>OpenClaw path (absolute)</label>
            <input v-model="localPath" placeholder="/home/user/.openclaw" />
          </div>
        </div>

        <!-- SSH tab -->
        <div v-if="sourceTab === 'ssh'" class="tab-content">
          <div class="field-row">
            <div class="field flex-2">
              <label>Host</label>
              <input v-model="sshHost" placeholder="192.168.1.50" />
            </div>
            <div class="field flex-1">
              <label>User (optional)</label>
              <input v-model="sshUser" placeholder="root" />
            </div>
            <div class="field flex-1">
              <label>Port</label>
              <input v-model="sshPort" placeholder="22" />
            </div>
          </div>
          <div class="field">
            <label>Remote path</label>
            <input v-model="sshRemotePath" placeholder="~/.openclaw" />
          </div>
          <div class="field-row">
            <div class="field flex-2">
              <label>Auth method</label>
              <select v-model="sshAuthMethod">
                <option value="agent">SSH Agent (recommended)</option>
                <option value="keyfile">Key File</option>
              </select>
            </div>
            <div v-if="sshAuthMethod === 'keyfile'" class="field flex-2">
              <label>Key path</label>
              <input v-model="sshKeyPath" placeholder="~/.ssh/id_ed25519" />
            </div>
          </div>
          <div class="ssh-test-row">
            <button class="secondary" @click="testSsh" :disabled="sshTesting || !sshHost.trim()">
              {{ sshTesting ? "Testing..." : "Test Connection" }}
            </button>
            <span v-if="sshTestResult" :class="sshTestResult.ok ? 'ssh-ok' : 'ssh-err'">
              <StatusDot :status="sshTestResult.ok ? 'ok' : 'error'" />
              {{ sshTestResult.message }}
            </span>
          </div>
        </div>
      </Card>

      <Card title="Import Preset">
        <div class="preset-list">
          <label class="preset" :class="{ selected: preset === 'minimal' }">
            <input type="radio" v-model="preset" value="minimal" />
            <div>
              <strong>Minimal</strong>
              <span class="dim">Workspaces + sessions only</span>
            </div>
          </label>
          <label class="preset" :class="{ selected: preset === 'full' }">
            <input type="radio" v-model="preset" value="full" />
            <div>
              <strong>Full</strong>
              <span class="dim">+ models.json (recommended)</span>
            </div>
          </label>
          <label class="preset" :class="{ selected: preset === 'everything' }">
            <input type="radio" v-model="preset" value="everything" />
            <div>
              <strong>Everything</strong>
              <span class="dim warn-text">+ auth-profiles.json (contains API keys)</span>
            </div>
          </label>
          <label class="preset" :class="{ selected: preset === 'custom' }">
            <input type="radio" v-model="preset" value="custom" />
            <div><strong>Custom</strong></div>
          </label>
        </div>

        <div v-if="preset === 'custom'" class="custom-opts">
          <label class="option"><input type="checkbox" v-model="customWorkspaces" /> Workspaces</label>
          <label class="option"><input type="checkbox" v-model="customSessions" /> Sessions</label>
          <label class="option"><input type="checkbox" v-model="customModels" /> Models</label>
          <label class="option warning-option"><input type="checkbox" v-model="customAuth" /> Auth profiles <span class="warn-text">(API keys)</span></label>
        </div>

        <div v-if="preset === 'everything' || (preset === 'custom' && customAuth)" class="auth-warning">
          <StatusDot status="warn" />
          <strong>Auth import enabled.</strong>
          auth-profiles.json contains plaintext API keys. Keys will be redacted in the preview but imported as-is.
        </div>
      </Card>

      <div class="action-bar">
        <button @click="doPreview" :disabled="!canScan">
          {{ scanning ? "Scanning..." : "Preview Import" }}
        </button>
      </div>
      <p v-if="scanError" class="error">{{ scanError }}</p>
    </template>

    <!-- ── STEP 2: Preview ─────────────────────────────────────── -->
    <template v-if="step === 'preview' && previewData">
      <!-- Inventory summary -->
      <Card title="Import Inventory">
        <div class="inv-summary">
          <div class="inv-stat">
            <span class="inv-num">{{ previewData.inventory.totals.approx_files }}</span>
            <span class="dim">files</span>
          </div>
          <div class="inv-stat">
            <span class="inv-num">{{ formatBytes(previewData.inventory.totals.approx_bytes) }}</span>
            <span class="dim">total size</span>
          </div>
          <div class="inv-stat">
            <span class="inv-num">{{ previewData.inventory.agents.length }}</span>
            <span class="dim">agents</span>
          </div>
          <div class="inv-stat">
            <span class="inv-num">{{ previewData.inventory.workspaces.length }}</span>
            <span class="dim">workspaces</span>
          </div>
        </div>

        <!-- Agents table -->
        <h4 v-if="previewData.inventory.agents.length" class="sub-heading">Agents</h4>
        <table v-if="previewData.inventory.agents.length" class="tbl">
          <thead>
            <tr><th>Agent ID</th><th>Sessions</th><th>Models</th><th>Auth</th></tr>
          </thead>
          <tbody>
            <tr v-for="a in previewData.inventory.agents" :key="a.agent_id">
              <td><code>{{ a.agent_id }}</code></td>
              <td>{{ a.session_files }} files</td>
              <td><StatusDot :status="a.has_models_json ? 'ok' : 'off'" /> {{ a.has_models_json ? "Yes" : "No" }}</td>
              <td><StatusDot :status="a.has_auth_profiles_json ? 'warn' : 'off'" /> {{ a.has_auth_profiles_json ? "Yes" : "No" }}</td>
            </tr>
          </tbody>
        </table>

        <!-- Workspaces table -->
        <h4 v-if="previewData.inventory.workspaces.length" class="sub-heading">Workspaces</h4>
        <table v-if="previewData.inventory.workspaces.length" class="tbl">
          <thead>
            <tr><th>Name</th><th>Files</th><th>Size</th></tr>
          </thead>
          <tbody>
            <tr v-for="w in previewData.inventory.workspaces" :key="w.rel_path">
              <td><code>{{ w.name }}</code></td>
              <td>{{ w.approx_files }}</td>
              <td class="dim">{{ formatBytes(w.approx_bytes) }}</td>
            </tr>
          </tbody>
        </table>
      </Card>

      <!-- Sensitive files warning -->
      <Card v-if="previewData.sensitive.sensitive_files.length" title="Sensitive Files Detected">
        <div class="sensitive-list">
          <div v-for="sf in previewData.sensitive.sensitive_files" :key="sf.rel_path" class="sensitive-row">
            <StatusDot status="warn" />
            <code>{{ sf.rel_path }}</code>
            <span class="dim">{{ sf.key_paths.join(", ") }}</span>
          </div>
        </div>
        <div v-if="previewData.sensitive.redacted_samples.length" class="redacted-samples">
          <h4 class="sub-heading">Redacted Key Samples</h4>
          <div v-for="s in previewData.sensitive.redacted_samples" :key="s" class="sample-row">
            <code>{{ s }}</code>
          </div>
        </div>
      </Card>

      <!-- Destinations -->
      <Card title="Destination">
        <div class="dest-info">
          <div><span class="label">Workspace</span> <code>{{ previewData.conflicts_hint.default_workspace_dest }}</code></div>
          <div><span class="label">Sessions</span> <code>{{ previewData.conflicts_hint.default_sessions_dest }}</code></div>
        </div>

        <h4 class="sub-heading" style="margin-top: 1rem">Merge Strategy</h4>
        <div class="strategy-list">
          <label class="strategy" :class="{ selected: mergeStrategy === 'merge_safe' }">
            <input type="radio" v-model="mergeStrategy" value="merge_safe" />
            <div>
              <strong>Safe Merge</strong>
              <span class="dim">Copy into imported/openclaw/... (no overwrite)</span>
            </div>
          </label>
          <label class="strategy" :class="{ selected: mergeStrategy === 'replace' }">
            <input type="radio" v-model="mergeStrategy" value="replace" />
            <div>
              <strong>Replace</strong>
              <span class="dim warn-text">Overwrite existing files/dirs</span>
            </div>
          </label>
          <label class="strategy" :class="{ selected: mergeStrategy === 'skip_existing' }">
            <input type="radio" v-model="mergeStrategy" value="skip_existing" />
            <div>
              <strong>Skip Existing</strong>
              <span class="dim">Only copy files that don't exist yet</span>
            </div>
          </label>
        </div>
      </Card>

      <div class="action-bar">
        <button @click="proceedToApply">Confirm &amp; Apply</button>
        <button class="secondary" @click="startOver">Back</button>
      </div>
    </template>

    <!-- ── STEP 3: Apply (progress) ────────────────────────────── -->
    <Card v-if="step === 'apply'" title="Applying Import">
      <div class="apply-confirm">
        <p>Ready to apply import with <strong>{{ strategyLabel[mergeStrategy] }}</strong> strategy.</p>
        <div v-if="previewData" class="apply-summary">
          <span>{{ previewData.inventory.agents.length }} agents</span>
          <span>{{ previewData.inventory.workspaces.length }} workspaces</span>
          <span>{{ previewData.inventory.totals.approx_files }} files</span>
        </div>
      </div>

      <div v-if="applying" class="progress-bar">
        <div class="progress-fill" style="width: 100%; animation: pulse 1.5s infinite"></div>
      </div>

      <div class="action-bar">
        <button @click="doApply" :disabled="applying">
          {{ applying ? "Importing..." : "Start Import" }}
        </button>
        <button class="secondary" @click="step = 'preview'" :disabled="applying">Back</button>
      </div>
      <p v-if="applyError" class="error">{{ applyError }}</p>
    </Card>

    <!-- ── STEP 4: Result ──────────────────────────────────────── -->
    <template v-if="step === 'result' && applyResult">
      <Card title="Import Complete">
        <div class="result-banner">
          <StatusDot status="ok" />
          <strong>Import successful</strong>
        </div>

        <div class="result-grid">
          <div v-if="applyResult.imported.workspaces.length">
            <span class="label">Workspaces</span>
            <code v-for="w in applyResult.imported.workspaces" :key="w" class="tag">{{ w }}</code>
          </div>
          <div v-if="applyResult.imported.agents.length">
            <span class="label">Agents</span>
            <code v-for="a in applyResult.imported.agents" :key="a" class="tag">{{ a }}</code>
          </div>
          <div>
            <span class="label">Sessions Copied</span>
            {{ applyResult.imported.sessions_copied }}
          </div>
          <div>
            <span class="label">Workspace Root</span>
            <code>{{ applyResult.imported.dest_workspace_root }}</code>
          </div>
          <div>
            <span class="label">Sessions Root</span>
            <code>{{ applyResult.imported.dest_sessions_root }}</code>
          </div>
        </div>

        <div v-if="applyResult.warnings.length" class="result-section">
          <h4 class="sub-heading">Warnings</h4>
          <p v-for="w in applyResult.warnings" :key="w" class="warn-line">
            <StatusDot status="warn" /> {{ w }}
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

/* ── Tabs ────────────────────────────────────────────────── */
.tab-bar {
  display: flex;
  gap: 0;
  margin-bottom: 1rem;
  border-bottom: 1px solid var(--border);
}
.tab-bar button {
  background: transparent;
  border: none;
  border-bottom: 2px solid transparent;
  color: var(--text-dim);
  padding: 0.5rem 1rem;
  cursor: pointer;
  font-size: 0.85rem;
}
.tab-bar button:hover { color: var(--text); }
.tab-bar .tab-active { color: var(--accent); border-bottom-color: var(--accent); }
.tab-content { padding-top: 0.5rem; }

/* ── Fields ──────────────────────────────────────────────── */
.field { display: flex; flex-direction: column; gap: 0.2rem; margin-bottom: 0.8rem; }
.field label { font-size: 0.78rem; color: var(--text-dim); }
.field input, .field select {
  background: var(--bg);
  border: 1px solid var(--border);
  color: var(--text);
  padding: 0.5rem 0.8rem;
  border-radius: 4px;
  font-family: var(--mono);
  font-size: 0.88rem;
  width: 100%;
}
.field-row { display: flex; gap: 0.8rem; }
.flex-1 { flex: 1; }
.flex-2 { flex: 2; }

.ssh-test-row { display: flex; align-items: center; gap: 0.8rem; margin-top: 0.3rem; }
.ssh-ok { color: var(--green); font-size: 0.85rem; }
.ssh-err { color: var(--red); font-size: 0.85rem; }

/* ── Presets ─────────────────────────────────────────────── */
.preset-list { display: flex; flex-direction: column; gap: 0.5rem; }
.preset {
  display: flex;
  align-items: flex-start;
  gap: 0.6rem;
  padding: 0.5rem 0.8rem;
  border: 1px solid var(--border);
  border-radius: 4px;
  cursor: pointer;
  font-size: 0.88rem;
}
.preset:hover { border-color: var(--text-dim); }
.preset.selected { border-color: var(--accent); background: rgba(88, 166, 255, 0.05); }
.preset div { display: flex; flex-direction: column; gap: 0.1rem; }
.preset input { margin-top: 0.15rem; cursor: pointer; }

.custom-opts { display: flex; flex-direction: column; gap: 0.4rem; margin-top: 0.8rem; padding-left: 0.5rem; }

/* ── Buttons ─────────────────────────────────────────────── */
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
button.secondary:hover:not(:disabled) { color: var(--text); border-color: var(--text-dim); }

.action-bar { display: flex; gap: 0.6rem; align-items: center; margin-top: 1rem; }
.secondary-link { font-size: 0.85rem; }

/* ── Options / warnings ──────────────────────────────────── */
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

/* ── Inventory ───────────────────────────────────────────── */
.inv-summary { display: flex; gap: 2rem; margin-bottom: 1rem; }
.inv-stat { display: flex; flex-direction: column; align-items: center; }
.inv-num { font-size: 1.6rem; font-weight: 700; color: var(--text); }

.tbl { width: 100%; border-collapse: collapse; font-size: 0.85rem; margin-bottom: 0.5rem; }
.tbl th { color: var(--text-dim); font-weight: 600; text-align: left; padding: 0.5rem 0.6rem; border-bottom: 1px solid var(--border); }
.tbl td { padding: 0.5rem 0.6rem; border-bottom: 1px solid var(--border); }

.sub-heading { color: var(--text-dim); font-size: 0.8rem; font-weight: 600; text-transform: uppercase; margin: 0.8rem 0 0.4rem; letter-spacing: 0.05em; }

/* ── Sensitive ───────────────────────────────────────────── */
.sensitive-list { display: flex; flex-direction: column; gap: 0.3rem; }
.sensitive-row { display: flex; align-items: center; gap: 0.5rem; font-size: 0.85rem; }
.redacted-samples { margin-top: 0.8rem; }
.sample-row { padding: 0.2rem 0; font-size: 0.82rem; color: var(--yellow); }

/* ── Destination / merge ─────────────────────────────────── */
.dest-info { font-size: 0.88rem; }
.dest-info div { margin: 0.3rem 0; }
.label { color: var(--text-dim); margin-right: 0.5em; font-size: 0.82rem; }

.strategy-list { display: flex; flex-direction: column; gap: 0.4rem; }
.strategy {
  display: flex;
  align-items: flex-start;
  gap: 0.6rem;
  padding: 0.4rem 0.8rem;
  border: 1px solid var(--border);
  border-radius: 4px;
  cursor: pointer;
  font-size: 0.88rem;
}
.strategy:hover { border-color: var(--text-dim); }
.strategy.selected { border-color: var(--accent); background: rgba(88, 166, 255, 0.05); }
.strategy div { display: flex; flex-direction: column; gap: 0.1rem; }
.strategy input { margin-top: 0.15rem; cursor: pointer; }

/* ── Apply progress ──────────────────────────────────────── */
.apply-confirm { margin-bottom: 1rem; font-size: 0.9rem; }
.apply-summary { display: flex; gap: 1.5rem; margin-top: 0.5rem; color: var(--text-dim); font-size: 0.85rem; }

.progress-bar {
  height: 4px;
  background: var(--border);
  border-radius: 2px;
  overflow: hidden;
  margin-bottom: 1rem;
}
.progress-fill {
  height: 100%;
  background: var(--accent);
  border-radius: 2px;
}
@keyframes pulse {
  0%, 100% { opacity: 0.4; }
  50% { opacity: 1; }
}

/* ── Result ──────────────────────────────────────────────── */
.result-banner { font-size: 1.1rem; margin-bottom: 1rem; }
.result-grid {
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
  margin-top: 0.5rem;
  font-size: 0.88rem;
}
.tag {
  display: inline-block;
  background: #21262d;
  padding: 0.15rem 0.5rem;
  border-radius: 3px;
  font-size: 0.8rem;
  margin: 0 0.2rem;
}
.result-section { margin-top: 1rem; }
.warn-line { color: var(--yellow); font-size: 0.85rem; margin: 0.2rem 0; }
</style>
