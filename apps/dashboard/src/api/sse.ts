/** Lightweight SSE helper for streaming run events. */

export interface SseOptions<T> {
  onEvent: (eventType: string, data: T) => void;
  onError?: (error: Event | string) => void;
  onClose?: () => void;
}

/**
 * Subscribe to an SSE endpoint. Returns a cleanup function.
 *
 * Usage:
 *   const unsub = subscribeSSE<RunEvent>("/v1/runs/abc/events", {
 *     onEvent(type, data) { ... },
 *     onError(e) { ... },
 *     onClose() { ... },
 *   });
 *   // Later:
 *   unsub();
 */
export function subscribeSSE<T = unknown>(
  path: string,
  opts: SseOptions<T>,
): () => void {
  const es = new EventSource(path);

  // Generic message handler (events without a named type)
  es.onmessage = (ev) => {
    try {
      const data = JSON.parse(ev.data) as T;
      opts.onEvent("message", data);
    } catch {
      opts.onEvent("message", ev.data as unknown as T);
    }
  };

  // Named event types we care about
  const eventTypes = [
    "run.status",
    "run.snapshot",
    "node.started",
    "node.completed",
    "node.failed",
    "log",
    "usage",
    "warning",
    "error",
    // Chat/turn events
    "assistant_delta",
    "tool_call",
    "tool_result",
    "final",
    "stopped",
  ];

  for (const type of eventTypes) {
    es.addEventListener(type, (ev: MessageEvent) => {
      try {
        const data = JSON.parse(ev.data) as T;
        opts.onEvent(type, data);
      } catch {
        opts.onEvent(type, ev.data as unknown as T);
      }
    });
  }

  es.onerror = (ev) => {
    if (es.readyState === EventSource.CLOSED) {
      opts.onClose?.();
    } else {
      opts.onError?.(ev);
    }
  };

  return () => {
    es.close();
  };
}
