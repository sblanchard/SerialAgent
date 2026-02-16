<script setup lang="ts">
import { ref, onMounted } from "vue";
import { api } from "@/api/client";
import type { AgentInfo } from "@/api/client";
import Card from "@/components/Card.vue";

const agents = ref<AgentInfo[]>([]);
const error = ref("");

async function load() {
  try {
    const res = await api.agents();
    agents.value = res.agents;
  } catch (e: any) {
    error.value = e.message;
  }
}

onMounted(load);
</script>

<template>
  <div>
    <h1 class="page-title">Agents</h1>
    <p v-if="error" class="error">{{ error }}</p>

    <Card v-if="agents.length === 0 && !error">
      <p class="dim" style="text-align:center;padding:2rem">No agents configured</p>
    </Card>

    <Card v-for="a in agents" :key="a.id" :title="a.id">
      <div class="detail-grid">
        <div><span class="label">Executor</span><code>{{ a.resolved_executor ?? "default" }}</code></div>
        <div><span class="label">Effective Tools</span>{{ a.effective_tools_count ?? "-" }}</div>
        <div><span class="label">Memory Mode</span><code>{{ a.memory_mode ?? "default" }}</code></div>
        <div><span class="label">Compaction</span>{{ a.compaction_enabled ? "enabled" : "disabled" }}</div>
        <div v-if="a.limits">
          <span class="label">Limits</span>
          depth={{ a.limits.max_depth }},
          children={{ a.limits.max_children_per_turn }},
          duration={{ a.limits.max_duration_ms }}ms
        </div>
      </div>

      <div v-if="a.tools_allow?.length" class="tag-row">
        <span class="label">Allow</span>
        <code v-for="t in a.tools_allow" :key="t" class="tag allow">{{ t }}</code>
      </div>
      <div v-if="a.tools_deny?.length" class="tag-row">
        <span class="label">Deny</span>
        <code v-for="t in a.tools_deny" :key="t" class="tag deny">{{ t }}</code>
      </div>
    </Card>
  </div>
</template>

<style scoped>
.page-title { font-size: 1.5rem; margin-bottom: 1.5rem; color: var(--accent); }
.error { color: var(--red); margin-bottom: 1rem; }
.dim { color: var(--text-dim); }
.detail-grid { display: grid; grid-template-columns: 1fr 1fr; gap: 0.5rem 2rem; font-size: 0.88rem; }
.label { color: var(--text-dim); margin-right: 0.5em; font-size: 0.82rem; }
.tag-row { margin-top: 0.6rem; display: flex; align-items: center; flex-wrap: wrap; gap: 0.3rem; }
.tag {
  display: inline-block;
  padding: 0.15rem 0.5rem;
  border-radius: 3px;
  font-size: 0.78rem;
}
.tag.allow { background: rgba(63, 185, 80, 0.15); color: var(--green); }
.tag.deny { background: rgba(248, 81, 73, 0.15); color: var(--red); }
</style>
