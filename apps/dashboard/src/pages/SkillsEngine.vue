<script setup lang="ts">
import { ref, onMounted } from "vue";
import { api } from "@/api/client";
import type { SkillEngineSpec, DangerLevel, ToolInvokeResponse } from "@/api/client";
import Card from "@/components/Card.vue";
import EmptyState from "@/components/EmptyState.vue";
import LoadingPanel from "@/components/LoadingPanel.vue";
import CodeAccordion from "@/components/CodeAccordion.vue";

const skills = ref<SkillEngineSpec[]>([]);
const loading = ref(true);
const error = ref("");

// Per-skill test panel state, keyed by skill name
const testInputs = ref<Record<string, string>>({});
const testRunning = ref<Record<string, boolean>>({});
const testResults = ref<Record<string, ToolInvokeResponse | null>>({});
const testErrors = ref<Record<string, string>>({});
const expandedTests = ref<Record<string, boolean>>({});

const dangerColors: Record<DangerLevel, string> = {
  safe: "var(--green)",
  network: "var(--accent)",
  filesystem: "var(--yellow)",
  execution: "var(--red)",
};

const dangerBg: Record<DangerLevel, string> = {
  safe: "rgba(63, 185, 80, 0.15)",
  network: "rgba(88, 166, 255, 0.15)",
  filesystem: "rgba(210, 153, 34, 0.15)",
  execution: "rgba(248, 81, 73, 0.15)",
};

async function load() {
  loading.value = true;
  error.value = "";
  try {
    const res = await api.getSkillEngine();
    skills.value = res.skills;
    // Initialize test inputs with empty JSON object for each skill
    for (const s of res.skills) {
      if (!(s.name in testInputs.value)) {
        testInputs.value[s.name] = "{}";
      }
    }
  } catch (e: any) {
    error.value = e.friendly ?? e.message;
  } finally {
    loading.value = false;
  }
}

function toggleTest(name: string) {
  expandedTests.value[name] = !expandedTests.value[name];
}

function formatSchema(schema: unknown): string {
  if (schema == null) return "null";
  return JSON.stringify(schema, null, 2);
}

async function runTest(skillName: string) {
  testRunning.value[skillName] = true;
  testErrors.value[skillName] = "";
  testResults.value[skillName] = null;

  let parsed: Record<string, unknown>;
  try {
    parsed = JSON.parse(testInputs.value[skillName] || "{}");
  } catch {
    testErrors.value[skillName] = "Invalid JSON in args input";
    testRunning.value[skillName] = false;
    return;
  }

  try {
    const result = await api.invokeTool({ tool: skillName, args: parsed });
    testResults.value[skillName] = result;
  } catch (e: any) {
    testErrors.value[skillName] = e.friendly ?? e.message;
  } finally {
    testRunning.value[skillName] = false;
  }
}

onMounted(load);
</script>

<template>
  <div>
    <h1 class="page-title">Skills Engine</h1>
    <p class="page-subtitle">
      Callable skills catalog. Browse available skills, inspect their schemas, and test invocations.
    </p>

    <p v-if="error" class="error">{{ error }}</p>

    <LoadingPanel v-if="loading" message="Loading skills catalog..." />

    <EmptyState
      v-else-if="skills.length === 0 && !error"
      title="No skills registered"
      description="The skill engine has no callable skills. Check your configuration and ensure skills are loaded."
    />

    <template v-else>
      <p class="skill-count dim">{{ skills.length }} skill{{ skills.length !== 1 ? "s" : "" }} available</p>

      <div class="skills-list">
        <Card v-for="skill in skills" :key="skill.name">
          <!-- Card header: name + danger badge -->
          <div class="skill-header">
            <div class="skill-identity">
              <code class="skill-name">{{ skill.name }}</code>
              <span
                class="danger-badge"
                :style="{
                  color: dangerColors[skill.danger_level],
                  background: dangerBg[skill.danger_level],
                }"
              >
                {{ skill.danger_level }}
              </span>
            </div>
            <span v-if="skill.title" class="skill-title">{{ skill.title }}</span>
          </div>

          <!-- Description -->
          <p v-if="skill.description" class="skill-desc">{{ skill.description }}</p>

          <!-- Schema accordions -->
          <div class="schema-section">
            <CodeAccordion
              label="Args Schema"
              :content="formatSchema(skill.args_schema)"
            />
            <CodeAccordion
              label="Returns Schema"
              :content="formatSchema(skill.returns_schema)"
            />
          </div>

          <!-- Test panel toggle -->
          <button class="test-toggle" @click="toggleTest(skill.name)">
            <span class="arrow" :class="{ expanded: expandedTests[skill.name] }">&#9654;</span>
            Test
          </button>

          <!-- Test panel -->
          <div v-if="expandedTests[skill.name]" class="test-panel">
            <div class="field">
              <label>Args (JSON)</label>
              <textarea
                v-model="testInputs[skill.name]"
                rows="4"
                spellcheck="false"
                placeholder='{ "key": "value" }'
              />
            </div>
            <button
              class="run-btn"
              :disabled="testRunning[skill.name]"
              @click="runTest(skill.name)"
            >
              {{ testRunning[skill.name] ? "Running..." : "Run" }}
            </button>

            <p v-if="testErrors[skill.name]" class="error" style="margin-top: 0.5rem">
              {{ testErrors[skill.name] }}
            </p>

            <div v-if="testResults[skill.name]" class="test-result">
              <div class="result-status">
                <span
                  class="result-dot"
                  :class="testResults[skill.name]!.ok ? 'dot-ok' : 'dot-error'"
                />
                <span>{{ testResults[skill.name]!.ok ? "Success" : "Error" }}</span>
                <span class="dim">
                  &mdash; {{ testResults[skill.name]!.duration_ms }}ms
                  via {{ testResults[skill.name]!.route.kind }}
                </span>
              </div>
              <CodeAccordion
                label="Result"
                :content="JSON.stringify(
                  testResults[skill.name]!.ok
                    ? testResults[skill.name]!.result
                    : testResults[skill.name]!.error,
                  null, 2
                )"
              />
            </div>
          </div>
        </Card>
      </div>
    </template>
  </div>
</template>

<style scoped>
.page-title {
  font-size: 1.5rem;
  margin-bottom: 0.3rem;
  color: var(--accent);
}
.page-subtitle {
  color: var(--text-dim);
  font-size: 0.88rem;
  margin-bottom: 1.5rem;
}

.error { color: var(--red); margin-bottom: 1rem; }
.dim { color: var(--text-dim); font-size: 0.85rem; }

.skill-count {
  margin-bottom: 1rem;
}

.skills-list {
  display: flex;
  flex-direction: column;
  gap: 0;
}

/* ── Skill card header ───────────────────────────────────────── */
.skill-header {
  display: flex;
  flex-direction: column;
  gap: 0.2rem;
  margin-bottom: 0.5rem;
}
.skill-identity {
  display: flex;
  align-items: center;
  gap: 0.6rem;
}
.skill-name {
  font-size: 0.95rem;
  font-weight: 600;
  color: var(--text);
}
.skill-title {
  font-size: 0.85rem;
  color: var(--text-dim);
}

/* ── Danger level badge ──────────────────────────────────────── */
.danger-badge {
  display: inline-block;
  padding: 0.1rem 0.5rem;
  border-radius: 3px;
  font-size: 0.72rem;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.04em;
}

/* ── Description ─────────────────────────────────────────────── */
.skill-desc {
  font-size: 0.85rem;
  color: var(--text-dim);
  margin-bottom: 0.8rem;
  line-height: 1.45;
}

/* ── Schema accordions section ───────────────────────────────── */
.schema-section {
  display: flex;
  flex-direction: column;
  gap: 0.4rem;
  margin-bottom: 0.6rem;
}

/* ── Test panel toggle ───────────────────────────────────────── */
.test-toggle {
  display: flex;
  align-items: center;
  gap: 0.4rem;
  background: transparent;
  border: 1px solid var(--border);
  border-radius: 4px;
  color: var(--text-dim);
  padding: 0.35rem 0.8rem;
  font-size: 0.78rem;
  cursor: pointer;
  transition: color 0.15s;
}
.test-toggle:hover { color: var(--text); }
.test-toggle .arrow {
  font-size: 0.6rem;
  transition: transform 0.15s;
}
.test-toggle .arrow.expanded { transform: rotate(90deg); }

/* ── Test panel ──────────────────────────────────────────────── */
.test-panel {
  margin-top: 0.6rem;
  padding: 0.8rem;
  border: 1px solid var(--border);
  border-radius: 4px;
  background: var(--bg);
}

.field {
  display: flex;
  flex-direction: column;
  gap: 0.2rem;
  margin-bottom: 0.6rem;
}
.field label {
  font-size: 0.78rem;
  color: var(--text-dim);
}
.field textarea {
  background: var(--bg-card);
  border: 1px solid var(--border);
  color: var(--text);
  padding: 0.4rem 0.6rem;
  border-radius: 4px;
  font-family: var(--mono);
  font-size: 0.85rem;
  resize: vertical;
}

.run-btn {
  background: var(--accent-dim);
  color: white;
  border: none;
  padding: 0.4rem 1.2rem;
  border-radius: 4px;
  cursor: pointer;
  font-size: 0.85rem;
}
.run-btn:disabled { opacity: 0.5; cursor: not-allowed; }
.run-btn:hover:not(:disabled) { background: var(--accent); }

/* ── Test result ─────────────────────────────────────────────── */
.test-result {
  margin-top: 0.8rem;
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
}
.result-status {
  display: flex;
  align-items: center;
  gap: 0.4rem;
  font-size: 0.85rem;
}
.result-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  flex-shrink: 0;
}
.dot-ok { background: var(--green); }
.dot-error { background: var(--red); }
</style>
