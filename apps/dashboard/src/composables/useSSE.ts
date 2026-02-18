import { ref, onUnmounted, type Ref } from "vue";
import { getApiToken } from "@/api/client";

// ── Types ──────────────────────────────────────────────────────────

export type SSEStatus = "connecting" | "open" | "closed" | "error";

export interface UseSSEOptions {
  /** Additional headers merged with the auth header. */
  headers?: Record<string, string>;
  /**
   * Named SSE event types to listen for in addition to the
   * generic `message` event. Defaults to listening only on `message`.
   */
  eventTypes?: string[];
  /** Initial backoff delay in milliseconds (default: 1000). */
  initialBackoffMs?: number;
  /** Maximum backoff delay in milliseconds (default: 30000). */
  maxBackoffMs?: number;
}

export interface UseSSEReturn<T> {
  /** Most recent parsed event data. */
  data: Ref<T | null>;
  /** Most recent error, if any. */
  error: Ref<string | null>;
  /** Current connection status. */
  status: Ref<SSEStatus>;
  /** Manually close the connection (prevents auto-reconnect). */
  close: () => void;
}

// ── Composable ─────────────────────────────────────────────────────

/**
 * Reactive Server-Sent Events composable with exponential backoff
 * reconnection, automatic auth header injection, and cleanup on
 * component unmount.
 *
 * Uses `EventSource` for native SSE and falls back to `fetch` with a
 * streaming reader when custom headers (like Authorization) are needed,
 * since the `EventSource` API does not support custom headers.
 */
export function useSSE<T = unknown>(
  url: string,
  options: UseSSEOptions = {},
): UseSSEReturn<T> {
  const {
    headers: extraHeaders = {},
    eventTypes = [],
    initialBackoffMs = 1_000,
    maxBackoffMs = 30_000,
  } = options;

  const data = ref<T | null>(null) as Ref<T | null>;
  const error = ref<string | null>(null) as Ref<string | null>;
  const status = ref<SSEStatus>("connecting") as Ref<SSEStatus>;

  let controller: AbortController | null = null;
  let backoff = initialBackoffMs;
  let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  let stopped = false;

  // ── Header builder ───────────────────────────────────────────────

  function buildHeaders(): Record<string, string> {
    const h: Record<string, string> = { ...extraHeaders };
    const token = getApiToken();
    if (token) {
      h["Authorization"] = `Bearer ${token}`;
    }
    return h;
  }

  // ── Determine whether custom headers are required ────────────────

  function needsCustomHeaders(): boolean {
    const h = buildHeaders();
    return Object.keys(h).length > 0;
  }

  // ── Parse SSE text frame ─────────────────────────────────────────

  function parseEventData(raw: string): void {
    try {
      const parsed = JSON.parse(raw) as T;
      data.value = parsed;
    } catch {
      // If the data is not JSON, store raw string as-is.
      data.value = raw as unknown as T;
    }
  }

  // ── Native EventSource path (no custom headers needed) ───────────

  function connectEventSource(): void {
    status.value = "connecting";
    error.value = null;

    const es = new EventSource(url);

    es.onopen = () => {
      status.value = "open";
      backoff = initialBackoffMs;
    };

    es.onmessage = (ev) => {
      parseEventData(ev.data as string);
    };

    for (const type of eventTypes) {
      es.addEventListener(type, ((ev: MessageEvent) => {
        parseEventData(ev.data as string);
      }) as EventListener);
    }

    es.onerror = () => {
      if (es.readyState === EventSource.CLOSED) {
        status.value = "closed";
        es.close();
        scheduleReconnect();
      } else {
        status.value = "error";
        error.value = "SSE connection error";
      }
    };

    // Store a synthetic controller so `close()` works uniformly.
    controller = new AbortController();
    const signal = controller.signal;
    signal.addEventListener("abort", () => {
      es.close();
    });
  }

  // ── Fetch-based SSE path (custom headers supported) ──────────────

  async function connectFetch(): Promise<void> {
    status.value = "connecting";
    error.value = null;

    controller = new AbortController();
    const { signal } = controller;

    try {
      const response = await fetch(url, {
        headers: buildHeaders(),
        signal,
      });

      if (!response.ok) {
        const body = await response.text().catch(() => "");
        throw new Error(
          `SSE request failed: ${response.status} ${body}`.trim(),
        );
      }

      status.value = "open";
      backoff = initialBackoffMs;

      const reader = response.body?.getReader();
      if (!reader) {
        throw new Error("Response body is not readable");
      }

      const decoder = new TextDecoder();
      let buffer = "";

      // eslint-disable-next-line no-constant-condition
      while (true) {
        const { done, value } = await reader.read();
        if (done) break;

        buffer += decoder.decode(value, { stream: true });
        const segments = buffer.split("\n\n");
        // The last element is the incomplete segment still accumulating.
        buffer = segments.pop() ?? "";

        for (const segment of segments) {
          const dataLine = segment
            .split("\n")
            .find((line) => line.startsWith("data: "));
          if (dataLine) {
            const raw = dataLine.slice(6);
            if (raw !== "[DONE]") {
              parseEventData(raw);
            }
          }
        }
      }

      // Stream ended naturally.
      status.value = "closed";
      scheduleReconnect();
    } catch (err: unknown) {
      if (signal.aborted) {
        // Intentional close; do not reconnect.
        status.value = "closed";
        return;
      }
      const message = err instanceof Error ? err.message : String(err);
      error.value = message;
      status.value = "error";
      scheduleReconnect();
    }
  }

  // ── Reconnect with exponential backoff ───────────────────────────

  function scheduleReconnect(): void {
    if (stopped) return;
    reconnectTimer = setTimeout(() => {
      reconnectTimer = null;
      backoff = Math.min(backoff * 2, maxBackoffMs);
      connect();
    }, backoff);
  }

  // ── Unified connect entry point ──────────────────────────────────

  function connect(): void {
    if (stopped) return;
    if (needsCustomHeaders()) {
      connectFetch();
    } else {
      connectEventSource();
    }
  }

  // ── Public close (disables auto-reconnect) ───────────────────────

  function close(): void {
    stopped = true;
    if (reconnectTimer !== null) {
      clearTimeout(reconnectTimer);
      reconnectTimer = null;
    }
    if (controller) {
      controller.abort();
      controller = null;
    }
    status.value = "closed";
  }

  // ── Lifecycle ────────────────────────────────────────────────────

  connect();

  onUnmounted(close);

  return { data, error, status, close };
}
