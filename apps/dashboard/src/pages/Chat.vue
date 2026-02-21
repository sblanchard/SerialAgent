<script setup lang="ts">
import { ref, nextTick, onMounted, onUnmounted } from "vue";
import { api, ApiError } from "@/api/client";
import type { Delivery } from "@/api/client";
import Card from "@/components/Card.vue";
import ThoughtBubble from "@/components/ThoughtBubble.vue";

type ChatMessage = {
  role: "user" | "assistant" | "tool_call" | "tool_result" | "error" | "thought" | "delivery";
  content: string;
  tool_name?: string;
  timestamp: string;
  delivery_id?: string;
  delivery_title?: string;
};

const messages = ref<ChatMessage[]>([]);
const input = ref("");
const sending = ref(false);
const sessionKey = ref("chat:dashboard:" + Date.now());
const streamingContent = ref("");
const isStreaming = ref(false);
const showThoughts = ref(localStorage.getItem("sa_show_thoughts") !== "false");
const thoughtBuffer = ref("");

// Track seen delivery IDs to avoid duplicates
const seenDeliveryIds = new Set<string>();
let deliveryPollTimer: ReturnType<typeof setInterval> | null = null;

function now(): string {
  return new Date().toLocaleTimeString();
}

function toggleThoughts() {
  showThoughts.value = !showThoughts.value;
  localStorage.setItem("sa_show_thoughts", String(showThoughts.value));
}

function flushThoughtBuffer() {
  if (thoughtBuffer.value) {
    messages.value.push({
      role: "thought",
      content: thoughtBuffer.value,
      timestamp: now(),
    });
    thoughtBuffer.value = "";
  }
}

async function loadUnreadDeliveries() {
  try {
    const data = await api.getDeliveries(10, 0);
    for (const d of data.deliveries) {
      if (d.read || seenDeliveryIds.has(d.id)) continue;
      seenDeliveryIds.add(d.id);
      messages.value.push({
        role: "delivery",
        content: d.body,
        timestamp: new Date(d.created_at).toLocaleTimeString(),
        delivery_id: d.id,
        delivery_title: d.schedule_name ?? "Scheduled Report",
      });
      // Mark as read
      api.markDeliveryRead(d.id).catch(() => {});
      scrollToBottom();
    }
  } catch {
    // Silently skip — deliveries are non-critical
  }
}

async function send() {
  const text = input.value.trim();
  if (!text || sending.value) return;

  messages.value.push({ role: "user", content: text, timestamp: now() });
  input.value = "";
  sending.value = true;
  streamingContent.value = "";
  isStreaming.value = true;

  await nextTick();
  scrollToBottom();

  try {
    const res = await fetch("/v1/chat/stream", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        session_key: sessionKey.value,
        message: text,
      }),
    });

    if (!res.ok) {
      const body = await res.text().catch(() => "");
      messages.value.push({
        role: "error",
        content: `Error ${res.status}: ${body}`,
        timestamp: now(),
      });
      sending.value = false;
      isStreaming.value = false;
      return;
    }

    const reader = res.body?.getReader();
    if (!reader) {
      sending.value = false;
      isStreaming.value = false;
      return;
    }

    const decoder = new TextDecoder();
    let buffer = "";

    while (true) {
      const { done, value } = await reader.read();
      if (done) break;

      buffer += decoder.decode(value, { stream: true });
      const lines = buffer.split("\n");
      buffer = lines.pop() || "";

      for (const line of lines) {
        if (!line.startsWith("data: ")) continue;
        const raw = line.slice(6);
        if (raw === "[DONE]") continue;

        try {
          const event = JSON.parse(raw);

          if (event.type === "thought") {
            thoughtBuffer.value += event.content;
            scrollToBottom();
          } else {
            flushThoughtBuffer();

            if (event.type === "assistant_delta") {
              streamingContent.value += event.text;
              scrollToBottom();
            } else if (event.type === "tool_call") {
              messages.value.push({
                role: "tool_call",
                content: JSON.stringify(event.arguments, null, 2),
                tool_name: event.tool_name,
                timestamp: now(),
              });
              scrollToBottom();
            } else if (event.type === "tool_result") {
              messages.value.push({
                role: "tool_result",
                content: event.content,
                tool_name: event.tool_name,
                timestamp: now(),
              });
              scrollToBottom();
            } else if (event.type === "final") {
              if (streamingContent.value) {
                messages.value.push({
                  role: "assistant",
                  content: event.content || streamingContent.value,
                  timestamp: now(),
                });
                streamingContent.value = "";
              }
            } else if (event.type === "error") {
              messages.value.push({
                role: "error",
                content: event.message,
                timestamp: now(),
              });
            }
          }
        } catch {
          // Skip unparseable events
        }
      }
    }

    flushThoughtBuffer();
    if (streamingContent.value) {
      messages.value.push({
        role: "assistant",
        content: streamingContent.value,
        timestamp: now(),
      });
      streamingContent.value = "";
    }
  } catch (e: unknown) {
    messages.value.push({
      role: "error",
      content: e instanceof Error ? e.message : String(e),
      timestamp: now(),
    });
  } finally {
    flushThoughtBuffer();
    sending.value = false;
    isStreaming.value = false;
  }
}

function scrollToBottom() {
  nextTick(() => {
    const el = document.querySelector(".chat-messages");
    if (el) el.scrollTop = el.scrollHeight;
  });
}

function newSession() {
  messages.value = [];
  seenDeliveryIds.clear();
  sessionKey.value = "chat:dashboard:" + Date.now();
  loadUnreadDeliveries();
}

function handleKeydown(e: KeyboardEvent) {
  if (e.key === "Enter" && !e.shiftKey) {
    e.preventDefault();
    send();
  }
}

onMounted(() => {
  loadUnreadDeliveries();
  // Poll for new deliveries every 30 seconds
  deliveryPollTimer = setInterval(loadUnreadDeliveries, 30_000);
});

onUnmounted(() => {
  if (deliveryPollTimer) clearInterval(deliveryPollTimer);
});
</script>

<template>
  <div class="chat-page">
    <div class="chat-header">
      <h1 class="page-title">Chat</h1>
      <button class="secondary" @click="newSession">New Session</button>
      <button class="secondary" @click="toggleThoughts">
        {{ showThoughts ? "Hide Thoughts" : "Show Thoughts" }}
      </button>
      <span class="dim session-id">{{ sessionKey.split(":").pop() }}</span>
    </div>

    <Card title="Conversation" class="chat-card">
      <div class="chat-messages">
        <div v-if="messages.length === 0 && !isStreaming" class="empty-chat dim">
          Send a message to start a conversation. Scheduled reports will appear here automatically.
        </div>

        <template v-for="(msg, i) in messages" :key="i">
          <ThoughtBubble
            v-if="msg.role === 'thought' && showThoughts"
            :content="msg.content"
            :timestamp="msg.timestamp"
          />
          <!-- Delivery notification -->
          <div v-else-if="msg.role === 'delivery'" class="chat-msg delivery">
            <div class="msg-header">
              <span class="msg-role delivery-badge">{{ msg.delivery_title }}</span>
              <span class="msg-time dim">{{ msg.timestamp }}</span>
            </div>
            <div class="msg-body">{{ msg.content }}</div>
          </div>
          <div
            v-else-if="msg.role !== 'thought'"
            class="chat-msg"
            :class="msg.role"
          >
            <div class="msg-header">
              <span class="msg-role">{{ msg.role === "tool_call" ? `tool: ${msg.tool_name}` : msg.role === "tool_result" ? `result: ${msg.tool_name}` : msg.role }}</span>
              <span class="msg-time dim">{{ msg.timestamp }}</span>
            </div>
            <div class="msg-body" :class="{ 'mono': msg.role === 'tool_call' || msg.role === 'tool_result' }">
              {{ msg.content }}
            </div>
          </div>
        </template>

        <!-- Streaming thought indicator -->
        <div v-if="isStreaming && thoughtBuffer && showThoughts" class="chat-msg thought-streaming">
          <div class="msg-header">
            <span class="msg-role">thinking</span>
            <span class="streaming-dot"></span>
          </div>
          <div class="msg-body thought-preview">{{ thoughtBuffer.slice(-200) }}</div>
        </div>

        <!-- Streaming indicator -->
        <div v-if="isStreaming && streamingContent" class="chat-msg assistant streaming">
          <div class="msg-header">
            <span class="msg-role">assistant</span>
            <span class="streaming-dot"></span>
          </div>
          <div class="msg-body">{{ streamingContent }}</div>
        </div>

        <div v-if="sending && !streamingContent && !thoughtBuffer" class="typing-indicator dim">
          Thinking...
        </div>
      </div>

      <div class="chat-input-bar">
        <textarea
          v-model="input"
          placeholder="Send a message... (Enter to send, Shift+Enter for newline)"
          class="chat-input"
          :disabled="sending"
          @keydown="handleKeydown"
          rows="2"
        ></textarea>
        <button class="send-btn" @click="send" :disabled="sending || !input.trim()">
          Send
        </button>
      </div>
    </Card>
  </div>
</template>

<style scoped>
.chat-page { display: flex; flex-direction: column; height: calc(100vh - 4rem); }
.chat-header {
  display: flex;
  align-items: center;
  gap: 0.8rem;
  margin-bottom: 1rem;
}
.page-title { font-size: 1.5rem; color: var(--accent); margin: 0; }
.dim { color: var(--text-dim); font-size: 0.85rem; }
.mono { font-family: var(--mono); font-size: 0.82rem; }
.session-id { font-family: var(--mono); font-size: 0.75rem; }

.chat-card { flex: 1; display: flex; flex-direction: column; min-height: 0; }

.chat-messages {
  flex: 1;
  overflow-y: auto;
  padding: 0.5rem 0;
  min-height: 200px;
  max-height: calc(100vh - 16rem);
}

.empty-chat {
  text-align: center;
  padding: 3rem 1rem;
}

.chat-msg {
  padding: 0.5rem 0.8rem;
  margin-bottom: 0.3rem;
  border-radius: 4px;
}
.chat-msg.user { background: rgba(88, 166, 255, 0.06); border-left: 2px solid var(--accent); }
.chat-msg.assistant { background: rgba(63, 185, 80, 0.04); border-left: 2px solid var(--green); }
.chat-msg.tool_call { background: rgba(210, 153, 34, 0.06); border-left: 2px solid #d29922; }
.chat-msg.tool_result { background: rgba(139, 148, 158, 0.06); border-left: 2px solid var(--text-dim); }
.chat-msg.error { background: rgba(248, 81, 73, 0.06); border-left: 2px solid var(--red); }
.chat-msg.streaming { opacity: 0.8; }
.chat-msg.delivery {
  background: rgba(163, 113, 247, 0.06);
  border-left: 2px solid #a371f7;
  margin-bottom: 0.6rem;
}

.delivery-badge {
  background: rgba(163, 113, 247, 0.15);
  color: #a371f7;
  padding: 0.1rem 0.5rem;
  border-radius: 3px;
  font-size: 0.72rem;
  font-weight: 700;
  text-transform: uppercase;
  letter-spacing: 0.04em;
}

.chat-msg.thought-streaming {
  background: rgba(139, 148, 158, 0.04);
  border-left: 2px solid var(--text-dim);
  opacity: 0.6;
}
.thought-preview {
  font-style: italic;
  color: var(--text-dim);
  font-size: 0.82rem;
  max-height: 2.8em;
  overflow: hidden;
}

.msg-header {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  margin-bottom: 0.2rem;
}
.msg-role {
  font-size: 0.72rem;
  font-weight: 700;
  text-transform: uppercase;
  letter-spacing: 0.04em;
  color: var(--text-dim);
}
.msg-time { font-size: 0.7rem; margin-left: auto; }
.msg-body {
  font-size: 0.85rem;
  white-space: pre-wrap;
  word-break: break-word;
  color: var(--text);
  max-height: 300px;
  overflow-y: auto;
}

.streaming-dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--green);
  animation: pulse-glow 1.5s ease-in-out infinite;
}
@keyframes pulse-glow {
  0%, 100% { opacity: 0.5; }
  50% { opacity: 1; }
}

.typing-indicator {
  padding: 0.5rem 0.8rem;
  font-style: italic;
}

.chat-input-bar {
  display: flex;
  gap: 0.5rem;
  padding-top: 0.8rem;
  border-top: 1px solid var(--border);
  margin-top: 0.5rem;
}
.chat-input {
  flex: 1;
  background: var(--bg);
  border: 1px solid var(--border);
  color: var(--text);
  padding: 0.5rem 0.8rem;
  border-radius: 4px;
  font-size: 0.85rem;
  font-family: inherit;
  resize: none;
}
.chat-input:focus { outline: none; border-color: var(--accent); }
.chat-input:disabled { opacity: 0.5; }

.send-btn {
  padding: 0.5rem 1.2rem;
  background: var(--accent);
  color: #fff;
  border: none;
  border-radius: 4px;
  font-size: 0.85rem;
  cursor: pointer;
  align-self: flex-end;
}
.send-btn:hover:not(:disabled) { opacity: 0.9; }
.send-btn:disabled { opacity: 0.4; cursor: not-allowed; }

button.secondary {
  background: transparent;
  border: 1px solid var(--border);
  color: var(--text-dim);
  padding: 0.3rem 0.8rem;
  border-radius: 4px;
  font-size: 0.78rem;
  cursor: pointer;
}
button.secondary:hover { color: var(--text); border-color: var(--text-dim); }
</style>
