<script setup lang="ts">
defineProps<{ status: "ok" | "warn" | "error" | "off" }>();
</script>

<template>
  <span :class="['dot', status]" />
</template>

<style scoped>
.dot {
  display: inline-block;
  width: 8px;
  height: 8px;
  border-radius: 50%;
  margin-right: 0.4em;
  vertical-align: middle;
}
.ok { background: var(--green); }
.warn { background: var(--yellow); }
.error { background: var(--red); }
.off { background: var(--text-dim); }
</style>
