/** Lightweight SSE helper for streaming run events with auth support. */

import { buildHeaders } from "./client";

export interface SseOptions<T> {
  onEvent: (eventType: string, data: T) => void;
  onError?: (error: Event | string) => void;
  onClose?: () => void;
}

/**
 * Subscribe to an SSE endpoint using fetch (supports Authorization headers).
 * Returns a cleanup function.
 */
export function subscribeSSE<T = unknown>(
  path: string,
  opts: SseOptions<T>,
): () => void {
  const controller = new AbortController();

  (async () => {
    try {
      const res = await fetch(path, {
        headers: buildHeaders(),
        signal: controller.signal,
      });

      if (!res.ok || !res.body) {
        opts.onError?.(`SSE connection failed: ${res.status}`);
        opts.onClose?.();
        return;
      }

      const reader = res.body.getReader();
      const decoder = new TextDecoder();
      let buffer = "";
      let currentEventType = "message";

      while (true) {
        const { done, value } = await reader.read();
        if (done) break;

        buffer += decoder.decode(value, { stream: true });
        const lines = buffer.split("\n");
        buffer = lines.pop() ?? "";

        for (const line of lines) {
          if (line.startsWith("event:")) {
            currentEventType = line.slice(6).trim();
          } else if (line.startsWith("data:")) {
            const raw = line.slice(5).trim();
            try {
              const data = JSON.parse(raw) as T;
              opts.onEvent(currentEventType, data);
            } catch {
              opts.onEvent(currentEventType, raw as unknown as T);
            }
            currentEventType = "message";
          } else if (line === "") {
            currentEventType = "message";
          }
        }
      }

      opts.onClose?.();
    } catch (err: unknown) {
      if (err instanceof DOMException && err.name === "AbortError") return;
      opts.onError?.(String(err));
      opts.onClose?.();
    }
  })();

  return () => {
    controller.abort();
  };
}
