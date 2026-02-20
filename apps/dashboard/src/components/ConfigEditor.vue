<script setup lang="ts">
import { ref, watch } from "vue";
import { downloadAsFile } from "@/utils/toml";
import { api } from "@/api/client";

const props = defineProps<{
  initialToml: string;
}>();

const content = ref(props.initialToml);
const saving = ref(false);
const saveStatus = ref<"idle" | "saved" | "error">("idle");
const saveMessage = ref("");

watch(
  () => props.initialToml,
  (v) => { content.value = v; },
);

function download() {
  downloadAsFile(content.value, "config.toml");
}

function reset() {
  content.value = props.initialToml;
  saveStatus.value = "idle";
}

function copyToClipboard() {
  navigator.clipboard.writeText(content.value);
}

async function save() {
  saving.value = true;
  saveStatus.value = "idle";
  try {
    const res = await api.saveConfig(content.value);
    saveStatus.value = "saved";
    saveMessage.value = res.note;
  } catch (e: unknown) {
    saveStatus.value = "error";
    saveMessage.value = e instanceof Error ? e.message : String(e);
  } finally {
    saving.value = false;
  }
}
</script>

<template>
  <div class="config-editor">
    <div class="editor-header">
      <span class="editor-label">config.toml</span>
      <div class="editor-actions">
        <button class="action-btn" @click="copyToClipboard">Copy</button>
        <button class="action-btn" @click="reset">Reset</button>
        <button class="action-btn" @click="download">Download</button>
        <button class="action-btn save" :disabled="saving" @click="save">
          {{ saving ? "Saving..." : "Save to Server" }}
        </button>
      </div>
    </div>
    <textarea
      v-model="content"
      class="editor-textarea"
      spellcheck="false"
    ></textarea>
    <div v-if="saveStatus === 'saved'" class="editor-hint status-ok">
      Saved. {{ saveMessage }}
    </div>
    <div v-else-if="saveStatus === 'error'" class="editor-hint status-err">
      Save failed: {{ saveMessage }}
    </div>
    <div v-else class="editor-hint dim">
      Edit the TOML above and save to server. A restart is required for changes to take effect.
    </div>
  </div>
</template>

<style scoped>
.config-editor {
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
}
.editor-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
}
.editor-label {
  font-family: var(--mono);
  font-size: 0.82rem;
  color: var(--text-dim);
  font-weight: 600;
}
.editor-actions {
  display: flex;
  gap: 0.4rem;
}
.action-btn {
  background: transparent;
  border: 1px solid var(--border);
  color: var(--text-dim);
  padding: 0.25rem 0.7rem;
  border-radius: 4px;
  font-size: 0.75rem;
  cursor: pointer;
}
.action-btn:hover {
  color: var(--text);
  border-color: var(--text-dim);
}
.action-btn.save {
  background: var(--accent);
  border-color: var(--accent);
  color: #fff;
}
.action-btn.save:hover {
  opacity: 0.9;
}
.action-btn.save:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
.editor-textarea {
  width: 100%;
  min-height: 400px;
  background: var(--bg);
  border: 1px solid var(--border);
  color: var(--text);
  font-family: var(--mono);
  font-size: 0.82rem;
  padding: 0.8rem;
  border-radius: 4px;
  resize: vertical;
  line-height: 1.5;
  tab-size: 4;
}
.editor-textarea:focus {
  outline: none;
  border-color: var(--accent);
}
.editor-hint {
  font-size: 0.75rem;
}
.dim { color: var(--text-dim); }
.status-ok { color: #4caf50; }
.status-err { color: #f44336; }
</style>
