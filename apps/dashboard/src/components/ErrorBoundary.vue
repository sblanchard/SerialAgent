<script setup lang="ts">
import { ref, onErrorCaptured } from "vue";

const props = defineProps<{
  /** Optional fallback title shown when an error is caught. */
  title?: string;
}>();

const emit = defineEmits<{
  (e: "error", error: Error, info: string): void;
}>();

const caughtError = ref<Error | null>(null);
const errorInfo = ref("");

onErrorCaptured((error: Error, _instance, info: string) => {
  caughtError.value = error;
  errorInfo.value = info;
  emit("error", error, info);

  // Returning false prevents the error from propagating further.
  return false;
});

function retry(): void {
  caughtError.value = null;
  errorInfo.value = "";
}
</script>

<template>
  <div v-if="caughtError" class="error-boundary">
    <div class="error-icon">!</div>
    <h3 class="error-title">{{ props.title || "Something went wrong" }}</h3>
    <p class="error-message">{{ caughtError.message }}</p>
    <p v-if="errorInfo" class="error-info">
      Component lifecycle: <code>{{ errorInfo }}</code>
    </p>
    <button class="retry-btn" @click="retry">Retry</button>
  </div>
  <slot v-else />
</template>

<style scoped>
.error-boundary {
  display: flex;
  flex-direction: column;
  align-items: center;
  padding: 2rem 1.5rem;
  background: var(--bg-card);
  border: 1px solid var(--red);
  border-radius: 6px;
  text-align: center;
}

.error-icon {
  width: 2.5rem;
  height: 2.5rem;
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: 50%;
  background: rgba(248, 81, 73, 0.15);
  color: var(--red);
  font-size: 1.2rem;
  font-weight: 700;
  font-family: var(--mono);
  margin-bottom: 0.8rem;
}

.error-title {
  color: var(--text);
  font-size: 1rem;
  font-weight: 600;
  margin: 0 0 0.5rem;
}

.error-message {
  color: var(--red);
  font-size: 0.85rem;
  margin: 0 0 0.4rem;
  max-width: 500px;
  word-break: break-word;
}

.error-info {
  color: var(--text-dim);
  font-size: 0.78rem;
  margin: 0 0 1rem;
}

.retry-btn {
  background: var(--accent-dim);
  color: white;
  border: none;
  padding: 0.45rem 1.2rem;
  border-radius: 4px;
  cursor: pointer;
  font-size: 0.85rem;
}

.retry-btn:hover {
  background: var(--accent);
}
</style>
