<script setup lang="ts">
defineProps<{
  icon?: string;
  title: string;
  description?: string;
}>();
</script>

<template>
  <div class="empty-state">
    <div v-if="icon" class="icon">{{ icon }}</div>
    <h3>{{ title }}</h3>
    <p v-if="description" class="desc">{{ description }}</p>
    <div class="actions">
      <slot />
    </div>
  </div>
</template>

<style scoped>
.empty-state {
  display: flex;
  flex-direction: column;
  align-items: center;
  padding: 2.5rem 1rem;
  color: var(--text-dim);
}
.icon { font-size: 2rem; margin-bottom: 0.5rem; font-family: var(--mono); }
h3 { font-size: 1rem; color: var(--text); margin: 0 0 0.3rem; }
.desc { font-size: 0.85rem; max-width: 400px; text-align: center; }
.actions { margin-top: 1rem; }
</style>
