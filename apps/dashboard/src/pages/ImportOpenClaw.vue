<script setup lang="ts">
import { ref } from "vue";
import { useOpenClawImport, IMPORT_LIMITS } from "@/composables/useOpenClawImport";
import type { MergeStrategy } from "@/api/client";
import Card from "@/components/Card.vue";
import StatusDot from "@/components/StatusDot.vue";

const {
  phase,
  sourceTab,
  localPath,
  sshHost,
  sshUser,
  sshPort,
  sshRemotePath,
  sshAuthMethod,
  sshKeyPath,
  sshTesting,
  sshTestResult,
  preset,
  customWorkspaces,
  customSessions,
  customModels,
  customAuth,
  canScan,
  isAuthEnabled,
  previewData,
  applyResult,
  mergeStrategy,
  error,
  testSsh,
  preview,
  confirmPreview,
  apply,
  reset,
  goBack,
  dismissError,
} = useOpenClawImport();

// Error detail toggle
const showErrorDetail = ref(false);

// Copy error detail to clipboard
function copyErrorDetail() {
  if (error.value) {
    navigator.clipboard.writeText(error.value.detail);
  }
}

// Step labels for the indicator
const STEPS = [
  { key: "source", label: "1. Source", phases: ["idle"] },
  { key: "validate", label: "2. Validate", phases: ["validating"] },
  { key: "preview", label: "3. Preview", phases: ["scanned", "previewing"] },
  { key: "apply", label: "4. Apply", phases: ["applying"] },
  { key: "done", label: "5. Done", phases: ["done"] },
] as const;

function isStepActive(phases: readonly string[]) {
  return phases.includes(phase.value) || (phase.value === "error");
}

function isStepDone(phases: readonly string[]) {
  const phaseOrder = ["idle", "validating", "scanned", "previewing", "applying", "done"];
  const currentIdx = phaseOrder.indexOf(phase.value);
  const stepIdx = Math.max(...phases.map((p) => phaseOrder.indexOf(p)));
  return currentIdx > stepIdx;
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
      <template v-for="(s, i) in STEPS" :key="s.key">
        <span v-if="i > 0" class="sep">&rarr;</span>
        <span
          :class="{
            active: isStepActive(s.phases),
            done: isStepDone(s.phases),
          }"
        >{{ s.label }}</span>
      </template>
    </div>

    <!-- ── Error panel (shown in any error state) ──────────────── -->
    <Card v-if="phase === 'error' && error" title="Import Failed">
      <div class="error-banner">
        <StatusDot status="error" />
        <strong>{{ error.friendly }}</strong>
        <span v-if="error.status" class="error-code">HTTP {{ error.status }}</span>
      </div>

      <div class="error-actions">
        <button class="detail-toggle" @click="showErrorDetail = !showErrorDetail">
          {{ showErrorDetail ? "Hide" : "Show" }} technical details
        </button>
        <button class="secondary copy-btn" @click="copyErrorDetail">Copy details</button>
      </div>

      <div v-if="showErrorDetail" class="error-detail">
        <code>{{ error.detail }}</code>
      </div>

      <div class="action-bar">
        <button @click="dismissError">Try Again</button>
        <button class="secondary" @click="reset">Start Over</button>
      </div>
    </Card>

    <!-- ── STEP A: Source ──────────────────────────────────────── -->
    <template v-if="phase === 'idle'">
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

        <div v-if="isAuthEnabled" class="auth-warning">
          <StatusDot status="warn" />
          <strong>Auth import enabled.</strong>
          auth-profiles.json contains plaintext API keys. Keys will be redacted in the preview but imported as-is.
        </div>
      </Card>

      <!-- Limits info -->
      <Card title="Archive Limits">
        <div class="limits-grid">
          <div class="limit-item">
            <span class="limit-label">Max archive size</span>
            <span class="limit-value">{{ formatBytes(IMPORT_LIMITS.maxTgzBytes) }}</span>
          </div>
          <div class="limit-item">
            <span class="limit-label">Max extracted size</span>
            <span class="limit-value">{{ formatBytes(IMPORT_LIMITS.maxExtractedBytes) }}</span>
          </div>
          <div class="limit-item">
            <span class="limit-label">Max file count</span>
            <span class="limit-value">{{ IMPORT_LIMITS.maxFileCount.toLocaleString() }}</span>
          </div>
          <div class="limit-item">
            <span class="limit-label">Max path depth</span>
            <span class="limit-value">{{ IMPORT_LIMITS.maxPathDepth }}</span>
          </div>
          <div class="limit-item">
            <span class="limit-label">Rejected types</span>
            <span class="limit-value dim">{{ IMPORT_LIMITS.rejectedTypes.join(", ") }}</span>
          </div>
        </div>
      </Card>

      <div class="action-bar">
        <button @click="preview" :disabled="!canScan">Preview Import</button>
        <router-link to="/staging" class="secondary-link">Manage staging</router-link>
      </div>
    </template>

    <!-- ── STEP B: Validating ──────────────────────────────────── -->
    <Card v-if="phase === 'validating'" title="Validating Archive">
      <div class="validate-status">
        <div class="spinner"></div>
        <div class="validate-text">
          <p>Validating archive contents...</p>
          <p class="dim">Checking paths, entry types, sizes, and security constraints</p>
        </div>
      </div>
    </Card>

    <!-- ── STEP C: Preview (scanned) ──────────────────────────── -->
    <template v-if="phase === 'scanned' && previewData">
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

      <!-- Destinations + merge -->
      <Card title="Destination &amp; Merge Strategy">
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
        <button @click="confirmPreview">Confirm &amp; Apply</button>
        <button class="secondary" @click="goBack">Back</button>
      </div>
    </template>

    <!-- ── STEP D: Apply (confirmation + progress) ─────────────── -->
    <Card v-if="phase === 'previewing' || phase === 'applying'" title="Apply Import">
      <div class="apply-confirm">
        <p>
          Apply import with <strong>{{ strategyLabel[mergeStrategy] }}</strong> strategy.
        </p>
        <div v-if="previewData" class="apply-summary">
          <span>{{ previewData.inventory.agents.length }} agents</span>
          <span>{{ previewData.inventory.workspaces.length }} workspaces</span>
          <span>{{ previewData.inventory.totals.approx_files }} files</span>
        </div>
      </div>

      <div v-if="phase === 'applying'" class="progress-bar">
        <div class="progress-fill" style="width: 100%; animation: pulse 1.5s infinite"></div>
      </div>

      <div class="action-bar">
        <button @click="apply" :disabled="phase === 'applying'">
          {{ phase === 'applying' ? "Importing..." : "Start Import" }}
        </button>
        <button class="secondary" @click="goBack" :disabled="phase === 'applying'">Back</button>
      </div>
    </Card>

    <!-- ── STEP E: Done ───────────────────────────────────────── -->
    <template v-if="phase === 'done' && applyResult">
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

        <div class="done-nav">
          <p class="dim">Staging data will be automatically cleaned up within 24 hours, or you can
            <router-link to="/staging">manage it manually</router-link>.</p>
        </div>

        <div class="action-bar">
          <button @click="reset">Import Another</button>
          <router-link to="/agents" class="secondary-link">View Agents</router-link>
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
.steps .done { color: var(--green); }
.steps .sep { color: var(--border); }

/* ── Error panel ──────────────────────────────────────────── */
.error-banner {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  font-size: 1rem;
  color: var(--red);
  margin-bottom: 0.8rem;
}
.error-code {
  margin-left: auto;
  font-size: 0.78rem;
  color: var(--text-dim);
  font-family: var(--mono);
}
.error-actions {
  display: flex;
  gap: 0.5rem;
  margin-bottom: 0.5rem;
}
.detail-toggle {
  background: transparent;
  border: 1px solid var(--border);
  color: var(--text-dim);
  font-size: 0.78rem;
  padding: 0.25rem 0.6rem;
}
.detail-toggle:hover { color: var(--text); border-color: var(--text-dim); }
.copy-btn { font-size: 0.78rem !important; padding: 0.25rem 0.6rem !important; }
.error-detail {
  background: var(--bg);
  border: 1px solid var(--border);
  border-radius: 4px;
  padding: 0.6rem 0.8rem;
  font-size: 0.82rem;
  color: var(--text-dim);
  word-break: break-all;
  margin-bottom: 0.5rem;
}

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

/* ── Limits ──────────────────────────────────────────────── */
.limits-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 0.4rem 1.5rem;
  font-size: 0.85rem;
}
.limit-item { display: flex; justify-content: space-between; padding: 0.25rem 0; }
.limit-label { color: var(--text-dim); }
.limit-value { font-family: var(--mono); }

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

/* ── Validation spinner ──────────────────────────────────── */
.validate-status {
  display: flex;
  align-items: center;
  gap: 1rem;
  padding: 1rem 0;
}
.validate-text p { margin: 0.2rem 0; }
.spinner {
  width: 24px;
  height: 24px;
  border: 3px solid var(--border);
  border-top-color: var(--accent);
  border-radius: 50%;
  animation: spin 0.8s linear infinite;
  flex-shrink: 0;
}
@keyframes spin { to { transform: rotate(360deg); } }

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

.done-nav {
  margin-top: 1rem;
  padding: 0.6rem 0.8rem;
  border: 1px solid var(--border);
  border-radius: 4px;
  font-size: 0.85rem;
}
</style>
