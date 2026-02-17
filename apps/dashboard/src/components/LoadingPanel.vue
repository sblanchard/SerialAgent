<script setup lang="ts">
defineProps<{
  message?: string;
}>();
</script>

<template>
  <div class="loading-panel">
    <div class="spinner"></div>
    <span class="msg">{{ message || "Loading..." }}</span>
  </div>
</template>

<style scoped>
.loading-panel {
  display: flex;
  align-items: center;
  gap: 0.8rem;
  padding: 1rem 0;
  color: var(--text-dim);
  font-size: 0.85rem;
}
.spinner {
  width: 20px;
  height: 20px;
  border: 3px solid var(--border);
  border-top-color: var(--accent);
  border-radius: 50%;
  animation: spin 0.8s linear infinite;
  flex-shrink: 0;
}
@keyframes spin { to { transform: rotate(360deg); } }
</style>
