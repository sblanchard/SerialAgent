<script setup lang="ts">
import { ref } from "vue";

defineProps<{
  content: string;
  timestamp: string;
}>();

const isExpanded = ref(false);
</script>

<template>
  <div class="chat-msg thought" :class="{ collapsed: !isExpanded }">
    <div class="msg-header" @click="isExpanded = !isExpanded">
      <span class="arrow" :class="{ expanded: isExpanded }">&#9654;</span>
      <span class="msg-role">thinking</span>
      <span class="msg-time dim">{{ timestamp }}</span>
    </div>
    <div class="msg-body thought-body">{{ content }}</div>
  </div>
</template>

<style scoped>
.thought {
  background: rgba(139, 148, 158, 0.04);
  border-left: 2px solid var(--text-dim);
  opacity: 0.7;
  padding: 0.5rem 0.8rem;
  margin-bottom: 0.3rem;
  border-radius: 4px;
}
.msg-header {
  display: flex;
  align-items: center;
  gap: 0.4rem;
  margin-bottom: 0.2rem;
  cursor: pointer;
  user-select: none;
}
.arrow {
  font-size: 0.55rem;
  transition: transform 0.15s;
  color: var(--text-dim);
}
.arrow.expanded { transform: rotate(90deg); }
.msg-role {
  font-size: 0.72rem;
  font-weight: 700;
  text-transform: uppercase;
  letter-spacing: 0.04em;
  color: var(--text-dim);
  font-style: italic;
}
.msg-time {
  font-size: 0.7rem;
  margin-left: auto;
}
.dim { color: var(--text-dim); }
.thought-body {
  font-size: 0.82rem;
  font-style: italic;
  color: var(--text-dim);
  white-space: pre-wrap;
  word-break: break-word;
}
.collapsed .thought-body {
  max-height: 1.4em;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>
