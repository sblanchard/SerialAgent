<script setup lang="ts">
defineProps<{
  status: "queued" | "running" | "completed" | "failed" | "stopped";
  size?: "sm" | "md";
}>();

const colors: Record<string, string> = {
  queued: "badge-dim",
  running: "badge-blue",
  completed: "badge-green",
  failed: "badge-red",
  stopped: "badge-yellow",
};
</script>

<template>
  <span class="badge" :class="[colors[status] || 'badge-dim', size === 'sm' ? 'sm' : '']">
    <span class="dot" :class="{ pulse: status === 'running' }"></span>
    {{ status }}
  </span>
</template>

<style scoped>
.badge {
  display: inline-flex;
  align-items: center;
  gap: 0.35rem;
  padding: 0.15rem 0.55rem;
  border-radius: 10px;
  font-size: 0.75rem;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.03em;
  border: 1px solid;
}
.badge.sm { font-size: 0.68rem; padding: 0.1rem 0.4rem; }

.dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  flex-shrink: 0;
}
.dot.pulse { animation: pulse-dot 1.5s ease-in-out infinite; }
@keyframes pulse-dot {
  0%, 100% { opacity: 0.4; }
  50% { opacity: 1; }
}

.badge-dim { color: var(--text-dim); border-color: var(--border); }
.badge-dim .dot { background: var(--text-dim); }

.badge-blue { color: var(--accent); border-color: var(--accent); background: rgba(88,166,255,0.08); }
.badge-blue .dot { background: var(--accent); }

.badge-green { color: var(--green); border-color: var(--green); background: rgba(63,185,80,0.08); }
.badge-green .dot { background: var(--green); }

.badge-red { color: var(--red); border-color: var(--red); background: rgba(248,81,73,0.08); }
.badge-red .dot { background: var(--red); }

.badge-yellow { color: var(--yellow); border-color: var(--yellow); background: rgba(210,153,34,0.08); }
.badge-yellow .dot { background: var(--yellow); }
</style>
