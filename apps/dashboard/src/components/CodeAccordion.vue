<script setup lang="ts">
import { ref } from "vue";

defineProps<{
  label: string;
  content: string;
}>();

const open = ref(false);

function copyContent(text: string) {
  navigator.clipboard.writeText(text);
}
</script>

<template>
  <div class="accordion">
    <button class="toggle" @click="open = !open">
      <span class="arrow" :class="{ expanded: open }">&#9654;</span>
      {{ label }}
    </button>
    <div v-if="open" class="body">
      <pre><code>{{ content }}</code></pre>
      <button class="copy-btn" @click="copyContent(content)">Copy</button>
    </div>
  </div>
</template>

<style scoped>
.accordion {
  border: 1px solid var(--border);
  border-radius: 4px;
  overflow: hidden;
}
.toggle {
  display: flex;
  align-items: center;
  gap: 0.4rem;
  width: 100%;
  padding: 0.4rem 0.8rem;
  background: transparent;
  border: none;
  color: var(--text-dim);
  font-size: 0.78rem;
  cursor: pointer;
  text-align: left;
}
.toggle:hover { color: var(--text); }
.arrow {
  font-size: 0.6rem;
  transition: transform 0.15s;
}
.arrow.expanded { transform: rotate(90deg); }

.body {
  padding: 0.5rem 0.8rem;
  border-top: 1px solid var(--border);
  background: var(--bg);
  position: relative;
}
pre {
  margin: 0;
  font-size: 0.78rem;
  color: var(--text-dim);
  white-space: pre-wrap;
  word-break: break-all;
  max-height: 300px;
  overflow-y: auto;
}
.copy-btn {
  position: absolute;
  top: 0.4rem;
  right: 0.4rem;
  padding: 0.15rem 0.5rem;
  font-size: 0.7rem;
  background: var(--bg-card);
  border: 1px solid var(--border);
  color: var(--text-dim);
  border-radius: 3px;
  cursor: pointer;
}
.copy-btn:hover { color: var(--text); }
</style>
