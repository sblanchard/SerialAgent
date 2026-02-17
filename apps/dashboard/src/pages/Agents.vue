<script setup lang="ts">
import { ref, onMounted } from "vue";
import { api } from "@/api/client";
import type {
  AgentInfo,
  WorkspaceFile,
  SkillDetailed,
} from "@/api/client";
import Card from "@/components/Card.vue";
import StatusDot from "@/components/StatusDot.vue";

const agents = ref<AgentInfo[]>([]);
const totalTools = ref(0);
const error = ref("");

// Workspace files
const wsFiles = ref<WorkspaceFile[]>([]);
const wsPath = ref("");
const wsError = ref("");

// Skills
const skills = ref<SkillDetailed[]>([]);
const skillsTotal = ref(0);
const skillsReady = ref(0);
const skillsError = ref("");

// Tabs for the detail panel
type Tab = "overview" | "files" | "tools" | "skills";
const activeTab = ref<Tab>("overview");
const selectedAgent = ref<AgentInfo | null>(null);

function selectAgent(a: AgentInfo) {
  selectedAgent.value = selectedAgent.value?.id === a.id ? null : a;
  activeTab.value = "overview";
}

async function load() {
  try {
    const res = await api.agents();
    agents.value = res.agents;
    totalTools.value = res.total_tools_available ?? 0;
  } catch (e: any) {
    error.value = e.message;
  }
}

async function loadWorkspace() {
  if (wsFiles.value.length > 0) return;
  wsError.value = "";
  try {
    const res = await api.workspaceFiles();
    wsFiles.value = res.files;
    wsPath.value = res.path;
  } catch (e: any) {
    wsError.value = e.message;
  }
}

async function loadSkills() {
  if (skills.value.length > 0) return;
  skillsError.value = "";
  try {
    const res = await api.skillsDetailed();
    skills.value = res.skills;
    skillsTotal.value = res.total;
    skillsReady.value = res.ready_count;
  } catch (e: any) {
    skillsError.value = e.message;
  }
}

function onTab(tab: Tab) {
  activeTab.value = tab;
  if (tab === "files") loadWorkspace();
  if (tab === "skills") loadSkills();
}

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  return `${(bytes / 1024).toFixed(1)} KB`;
}

const riskColor: Record<string, string> = {
  Pure: "var(--green)",
  Io: "var(--accent)",
  Net: "var(--yellow)",
  Admin: "var(--red)",
};

onMounted(load);
</script>

<template>
  <div>
    <h1 class="page-title">
      Agents
      <router-link to="/import" class="import-link">+ Import OpenClaw</router-link>
    </h1>
    <p v-if="error" class="error">{{ error }}</p>

    <!-- Agent list -->
    <Card>
      <table class="tbl">
        <thead>
          <tr>
            <th>Agent ID</th>
            <th>Executor</th>
            <th>Tools</th>
            <th>Memory</th>
            <th>Compaction</th>
          </tr>
        </thead>
        <tbody>
          <tr
            v-for="a in agents" :key="a.id"
            class="clickable"
            :class="{ 'selected-row': selectedAgent?.id === a.id }"
            @click="selectAgent(a)"
          >
            <td><code>{{ a.id }}</code></td>
            <td class="dim"><code>{{ a.resolved_executor ?? "default" }}</code></td>
            <td>{{ a.effective_tools_count ?? "-" }}<span class="dim"> / {{ totalTools }}</span></td>
            <td><code>{{ a.memory_mode ?? "default" }}</code></td>
            <td>
              <StatusDot :status="a.compaction_enabled ? 'ok' : 'off'" />
              {{ a.compaction_enabled ? "on" : "off" }}
            </td>
          </tr>
          <tr v-if="agents.length === 0 && !error">
            <td colspan="5" class="dim" style="text-align:center;padding:2rem">
              No agents configured.
              <router-link to="/import">Import from OpenClaw?</router-link>
            </td>
          </tr>
        </tbody>
      </table>
    </Card>

    <!-- Detail panel with tabs -->
    <div v-if="selectedAgent" class="detail">
      <div class="tabs">
        <button :class="{ active: activeTab === 'overview' }" @click="onTab('overview')">Overview</button>
        <button :class="{ active: activeTab === 'files' }" @click="onTab('files')">Files</button>
        <button :class="{ active: activeTab === 'tools' }" @click="onTab('tools')">Tools</button>
        <button :class="{ active: activeTab === 'skills' }" @click="onTab('skills')">Skills</button>
      </div>

      <!-- Overview tab -->
      <Card v-if="activeTab === 'overview'" :title="selectedAgent.id">
        <div class="detail-grid">
          <div><span class="label">Executor</span><code>{{ selectedAgent.resolved_executor ?? "default" }}</code></div>
          <div><span class="label">Memory Mode</span><code>{{ selectedAgent.memory_mode ?? "default" }}</code></div>
          <div><span class="label">Effective Tools</span>{{ selectedAgent.effective_tools_count ?? "-" }} of {{ totalTools }}</div>
          <div><span class="label">Compaction</span>{{ selectedAgent.compaction_enabled ? "enabled" : "disabled" }}</div>
        </div>

        <div v-if="selectedAgent.limits" style="margin-top: 0.8rem">
          <h4 class="sub-heading">Limits</h4>
          <div class="detail-grid">
            <div><span class="label">Max Depth</span>{{ selectedAgent.limits.max_depth }}</div>
            <div><span class="label">Max Children/Turn</span>{{ selectedAgent.limits.max_children_per_turn }}</div>
            <div><span class="label">Max Duration</span>{{ selectedAgent.limits.max_duration_ms }}ms</div>
          </div>
        </div>

        <div v-if="selectedAgent.models && Object.keys(selectedAgent.models).length" style="margin-top: 0.8rem">
          <h4 class="sub-heading">Model Assignments</h4>
          <div v-for="(model, role) in selectedAgent.models" :key="role" class="model-row">
            <span class="label">{{ role }}</span><code>{{ model }}</code>
          </div>
        </div>

        <div v-if="selectedAgent.tools_allow?.length" class="tag-row">
          <span class="label">Allow</span>
          <code v-for="t in selectedAgent.tools_allow" :key="t" class="tag allow">{{ t }}</code>
        </div>
        <div v-if="selectedAgent.tools_deny?.length" class="tag-row">
          <span class="label">Deny</span>
          <code v-for="t in selectedAgent.tools_deny" :key="t" class="tag deny">{{ t }}</code>
        </div>
      </Card>

      <!-- Files tab -->
      <Card v-if="activeTab === 'files'" title="Workspace Files">
        <p v-if="wsError" class="error">{{ wsError }}</p>
        <p class="dim" style="margin-bottom:0.6rem">Path: <code>{{ wsPath }}</code></p>
        <table class="tbl" v-if="wsFiles.length">
          <thead><tr><th>File</th><th>Size</th><th>SHA-256</th></tr></thead>
          <tbody>
            <tr v-for="f in wsFiles" :key="f.name">
              <td><code>{{ f.name }}</code></td>
              <td class="dim">{{ formatSize(f.size) }}</td>
              <td class="dim"><code>{{ f.sha256?.slice(0, 12) ?? "n/a" }}...</code></td>
            </tr>
          </tbody>
        </table>
        <p v-else class="dim">No workspace files</p>
      </Card>

      <!-- Tools tab -->
      <Card v-if="activeTab === 'tools'" title="Tool Policy">
        <div v-if="selectedAgent.tools_allow?.length" class="tag-section">
          <h4 class="sub-heading">Allowed Patterns</h4>
          <div class="tag-wrap">
            <code v-for="t in selectedAgent.tools_allow" :key="t" class="tag allow">{{ t }}</code>
          </div>
        </div>
        <div v-if="selectedAgent.tools_deny?.length" class="tag-section">
          <h4 class="sub-heading">Denied Patterns</h4>
          <div class="tag-wrap">
            <code v-for="t in selectedAgent.tools_deny" :key="t" class="tag deny">{{ t }}</code>
          </div>
        </div>
        <p class="dim" style="margin-top:0.8rem">
          Effective tools: <strong>{{ selectedAgent.effective_tools_count }}</strong> / {{ totalTools }}
        </p>
      </Card>

      <!-- Skills tab -->
      <Card v-if="activeTab === 'skills'" title="Skills">
        <p v-if="skillsError" class="error">{{ skillsError }}</p>
        <p class="dim" style="margin-bottom:0.6rem">{{ skillsReady }} of {{ skillsTotal }} ready</p>
        <table class="tbl" v-if="skills.length">
          <thead><tr><th></th><th>Skill</th><th>Risk</th><th>Description</th></tr></thead>
          <tbody>
            <tr v-for="s in skills" :key="s.name">
              <td><StatusDot :status="s.ready ? 'ok' : 'off'" /></td>
              <td><code>{{ s.name }}</code></td>
              <td :style="{ color: riskColor[s.risk] ?? 'var(--text)' }">{{ s.risk }}</td>
              <td class="dim">{{ s.description }}</td>
            </tr>
          </tbody>
        </table>
        <p v-else class="dim">No skills loaded</p>
      </Card>
    </div>
  </div>
</template>

<style scoped>
.page-title {
  font-size: 1.5rem;
  margin-bottom: 1.5rem;
  color: var(--accent);
  display: flex;
  align-items: center;
  gap: 1rem;
}
.import-link {
  font-size: 0.8rem;
  padding: 0.3rem 0.8rem;
  background: rgba(88, 166, 255, 0.1);
  border: 1px solid var(--accent-dim);
  border-radius: 4px;
  color: var(--accent);
}
.import-link:hover { background: rgba(88, 166, 255, 0.2); text-decoration: none; }

.error { color: var(--red); margin-bottom: 1rem; }
.dim { color: var(--text-dim); font-size: 0.85rem; }
.tbl { width: 100%; border-collapse: collapse; font-size: 0.85rem; }
.tbl th { color: var(--text-dim); font-weight: 600; text-align: left; padding: 0.5rem 0.6rem; border-bottom: 1px solid var(--border); }
.tbl td { padding: 0.5rem 0.6rem; border-bottom: 1px solid var(--border); }
.clickable { cursor: pointer; }
.clickable:hover { background: rgba(88, 166, 255, 0.05); }
.selected-row { background: rgba(88, 166, 255, 0.1) !important; }

.detail { margin-top: 1rem; }
.tabs {
  display: flex;
  gap: 0;
  margin-bottom: 1rem;
  border-bottom: 1px solid var(--border);
}
.tabs button {
  background: none;
  border: none;
  color: var(--text-dim);
  padding: 0.5rem 1rem;
  font-size: 0.85rem;
  cursor: pointer;
  border-bottom: 2px solid transparent;
  transition: all 0.15s;
}
.tabs button:hover { color: var(--text); }
.tabs button.active {
  color: var(--accent);
  border-bottom-color: var(--accent);
}

.detail-grid { display: grid; grid-template-columns: 1fr 1fr; gap: 0.5rem 2rem; font-size: 0.88rem; }
.label { color: var(--text-dim); margin-right: 0.5em; font-size: 0.82rem; }
.sub-heading { color: var(--text-dim); font-size: 0.8rem; font-weight: 600; text-transform: uppercase; margin: 0.8rem 0 0.4rem; letter-spacing: 0.05em; }
.model-row { font-size: 0.88rem; margin: 0.2rem 0; }

.tag-row, .tag-section { margin-top: 0.6rem; }
.tag-wrap { display: flex; flex-wrap: wrap; gap: 0.3rem; }
.tag {
  display: inline-block;
  padding: 0.15rem 0.5rem;
  border-radius: 3px;
  font-size: 0.78rem;
  margin: 0 0.2rem;
}
.tag.allow { background: rgba(63, 185, 80, 0.15); color: var(--green); }
.tag.deny { background: rgba(248, 81, 73, 0.15); color: var(--red); }
</style>
