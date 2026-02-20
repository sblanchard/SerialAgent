<script setup lang="ts">
import { ref, computed, watch, onMounted, onUnmounted } from "vue";

const props = defineProps<{
  modelValue: string;
}>();

const emit = defineEmits<{
  "update:modelValue": [value: string];
}>();

const query = ref("");
const open = ref(false);
const highlightIndex = ref(-1);
const wrapperRef = ref<HTMLElement | null>(null);
const listRef = ref<HTMLElement | null>(null);

// ── Build timezone list with UTC offsets ──────────────────────────

interface TzEntry {
  name: string;
  offset: string;
  offsetMinutes: number;
}

function buildTimezoneList(): TzEntry[] {
  let zones: string[];
  try {
    zones = (Intl as unknown as { supportedValuesOf(key: string): string[] })
      .supportedValuesOf("timeZone");
  } catch {
    // Fallback for older browsers
    zones = [
      "UTC", "US/Eastern", "US/Central", "US/Mountain", "US/Pacific",
      "America/New_York", "America/Chicago", "America/Denver", "America/Los_Angeles",
      "America/Toronto", "America/Sao_Paulo", "Europe/London", "Europe/Paris",
      "Europe/Berlin", "Europe/Moscow", "Asia/Tokyo", "Asia/Shanghai",
      "Asia/Kolkata", "Asia/Singapore", "Australia/Sydney", "Pacific/Auckland",
    ];
  }

  const now = new Date();
  return zones.map((name) => {
    try {
      const parts = new Intl.DateTimeFormat("en-US", {
        timeZone: name,
        timeZoneName: "shortOffset",
      }).formatToParts(now);
      const tzPart = parts.find((p) => p.type === "timeZoneName");
      const offsetStr = tzPart?.value ?? "";
      // Parse offset like "GMT+5:30" or "GMT-8" to minutes
      const match = offsetStr.match(/GMT([+-]?)(\d+)(?::(\d+))?/);
      let offsetMinutes = 0;
      if (match) {
        const sign = match[1] === "-" ? -1 : 1;
        offsetMinutes = sign * (parseInt(match[2]) * 60 + parseInt(match[3] || "0"));
      }
      return { name, offset: offsetStr || "GMT", offsetMinutes };
    } catch {
      return { name, offset: "GMT", offsetMinutes: 0 };
    }
  });
}

const allTimezones = buildTimezoneList();

const filtered = computed(() => {
  const q = query.value.toLowerCase().trim();
  if (!q) return allTimezones;
  return allTimezones.filter(
    (tz) =>
      tz.name.toLowerCase().includes(q) ||
      tz.offset.toLowerCase().includes(q)
  );
});

// ── Display label ────────────────────────────────────────────────

const displayLabel = computed(() => {
  const tz = allTimezones.find((t) => t.name === props.modelValue);
  if (!tz) return props.modelValue || "Select timezone";
  return `${tz.name} (${tz.offset})`;
});

// ── Interactions ─────────────────────────────────────────────────

function toggleOpen() {
  open.value = !open.value;
  if (open.value) {
    query.value = "";
    highlightIndex.value = -1;
  }
}

function select(tz: TzEntry) {
  emit("update:modelValue", tz.name);
  open.value = false;
  query.value = "";
}

function onKeydown(e: KeyboardEvent) {
  const list = filtered.value;
  if (e.key === "ArrowDown") {
    e.preventDefault();
    highlightIndex.value = Math.min(highlightIndex.value + 1, list.length - 1);
    scrollToHighlighted();
  } else if (e.key === "ArrowUp") {
    e.preventDefault();
    highlightIndex.value = Math.max(highlightIndex.value - 1, 0);
    scrollToHighlighted();
  } else if (e.key === "Enter" && highlightIndex.value >= 0 && highlightIndex.value < list.length) {
    e.preventDefault();
    select(list[highlightIndex.value]);
  } else if (e.key === "Escape") {
    open.value = false;
  }
}

function scrollToHighlighted() {
  if (!listRef.value) return;
  const item = listRef.value.children[highlightIndex.value] as HTMLElement | undefined;
  item?.scrollIntoView({ block: "nearest" });
}

// Reset highlight when query changes
watch(query, () => {
  highlightIndex.value = filtered.value.length > 0 ? 0 : -1;
});

// Close on outside click
function onClickOutside(e: MouseEvent) {
  if (wrapperRef.value && !wrapperRef.value.contains(e.target as Node)) {
    open.value = false;
  }
}

onMounted(() => document.addEventListener("mousedown", onClickOutside));
onUnmounted(() => document.removeEventListener("mousedown", onClickOutside));
</script>

<template>
  <div ref="wrapperRef" class="tz-picker">
    <button type="button" class="tz-trigger" @click="toggleOpen">
      <span class="tz-label">{{ displayLabel }}</span>
      <span class="tz-arrow">{{ open ? "\u25B2" : "\u25BC" }}</span>
    </button>

    <div v-if="open" class="tz-dropdown">
      <input
        v-model="query"
        class="tz-search"
        type="text"
        placeholder="Search timezones..."
        autofocus
        @keydown="onKeydown"
      />
      <ul ref="listRef" class="tz-list" role="listbox">
        <li
          v-for="(tz, i) in filtered"
          :key="tz.name"
          class="tz-item"
          :class="{ highlighted: i === highlightIndex, selected: tz.name === modelValue }"
          role="option"
          @click="select(tz)"
          @mouseenter="highlightIndex = i"
        >
          <span class="tz-name">{{ tz.name }}</span>
          <span class="tz-offset">{{ tz.offset }}</span>
        </li>
        <li v-if="filtered.length === 0" class="tz-empty">No matching timezones</li>
      </ul>
    </div>
  </div>
</template>

<style scoped>
.tz-picker {
  position: relative;
  width: 100%;
}

.tz-trigger {
  display: flex;
  align-items: center;
  justify-content: space-between;
  width: 100%;
  background: var(--bg);
  border: 1px solid var(--border);
  color: var(--text);
  padding: 0.5rem 0.8rem;
  border-radius: 4px;
  font-family: var(--mono);
  font-size: 0.88rem;
  cursor: pointer;
  text-align: left;
  box-sizing: border-box;
}

.tz-trigger:hover {
  border-color: var(--text-dim);
}

.tz-arrow {
  font-size: 0.6rem;
  color: var(--text-dim);
  margin-left: 0.5rem;
  flex-shrink: 0;
}

.tz-dropdown {
  position: absolute;
  top: calc(100% + 2px);
  left: 0;
  right: 0;
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: 4px;
  z-index: 100;
  box-shadow: 0 4px 12px rgba(0, 0, 0, 0.3);
}

.tz-search {
  display: block;
  width: 100%;
  background: var(--bg);
  border: none;
  border-bottom: 1px solid var(--border);
  color: var(--text);
  padding: 0.5rem 0.8rem;
  font-family: var(--mono);
  font-size: 0.85rem;
  outline: none;
  box-sizing: border-box;
}

.tz-search::placeholder {
  color: var(--text-dim);
}

.tz-list {
  list-style: none;
  margin: 0;
  padding: 0;
  max-height: 240px;
  overflow-y: auto;
}

.tz-item {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 0.35rem 0.8rem;
  cursor: pointer;
  font-size: 0.82rem;
}

.tz-item.highlighted {
  background: rgba(88, 166, 255, 0.1);
}

.tz-item.selected {
  color: var(--accent);
}

.tz-name {
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.tz-offset {
  color: var(--text-dim);
  font-size: 0.75rem;
  margin-left: 0.8rem;
  flex-shrink: 0;
}

.tz-empty {
  padding: 0.6rem 0.8rem;
  color: var(--text-dim);
  font-size: 0.82rem;
  text-align: center;
}
</style>
