<script setup lang="ts">
import { ref, watch } from "vue";
import { downloadAsFile } from "@/utils/toml";

const props = defineProps<{
  initialToml: string;
}>();

const content = ref(props.initialToml);

watch(
  () => props.initialToml,
  (v) => { content.value = v; },
);

function download() {
  downloadAsFile(content.value, "config.toml");
}

function reset() {
  content.value = props.initialToml;
}

function copyToClipboard() {
  navigator.clipboard.writeText(content.value);
}
</script>

<template>
  <div class="config-editor">
    <div class="editor-header">
      <span class="editor-label">config.toml</span>
      <div class="editor-actions">
        <button class="action-btn" @click="copyToClipboard">Copy</button>
        <button class="action-btn" @click="reset">Reset</button>
        <button class="action-btn primary" @click="download">Download</button>
      </div>
    </div>
    <textarea
      v-model="content"
      class="editor-textarea"
      spellcheck="false"
    ></textarea>
    <div class="editor-hint dim">
      Edit the TOML above, then download to replace your config.toml. A server restart is required for changes to take effect.
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
.action-btn.primary {
  background: var(--accent);
  border-color: var(--accent);
  color: #fff;
}
.action-btn.primary:hover {
  opacity: 0.9;
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
</style>
