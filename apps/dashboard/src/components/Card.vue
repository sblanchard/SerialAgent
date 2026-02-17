<script setup lang="ts">
defineProps<{ title?: string }>();
</script>

<template>
  <div class="card">
    <h3 v-if="title" class="card-title">{{ title }}</h3>
    <slot />
  </div>
</template>

<style scoped>
.card {
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: 6px;
  padding: 1rem 1.2rem;
  margin-bottom: 1rem;
}
.card-title {
  color: var(--accent);
  font-size: 0.85rem;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.05em;
  margin-bottom: 0.8rem;
}
</style>
