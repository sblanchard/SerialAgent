import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { subscribeSSE } from "../sse";
import type { SseOptions } from "../sse";

// ── Mock fetch with ReadableStream ──────────────────────────────────

function createMockStream(chunks: string[]) {
  const encoder = new TextEncoder();
  let index = 0;
  return new ReadableStream<Uint8Array>({
    pull(controller) {
      if (index < chunks.length) {
        controller.enqueue(encoder.encode(chunks[index]));
        index++;
      } else {
        controller.close();
      }
    },
  });
}

function mockFetchResponse(chunks: string[], status = 200) {
  const body = createMockStream(chunks);
  return vi.fn().mockResolvedValue({
    ok: status >= 200 && status < 300,
    status,
    body,
  } as unknown as Response);
}

beforeEach(() => {
  vi.stubGlobal("fetch", mockFetchResponse([]));
});

afterEach(() => {
  vi.unstubAllGlobals();
});

// ── Tests ───────────────────────────────────────────────────────────

describe("subscribeSSE", () => {
  it("returns a cleanup function", () => {
    const unsub = subscribeSSE("/v1/test", { onEvent: vi.fn() });
    expect(typeof unsub).toBe("function");
    unsub();
  });

  it("calls onEvent with parsed JSON data lines", async () => {
    const onEvent = vi.fn();
    const onClose = vi.fn();

    vi.stubGlobal(
      "fetch",
      mockFetchResponse(["data: {\"id\":1}\n\n"]),
    );

    subscribeSSE("/v1/test", { onEvent, onClose });

    // Allow the async stream reader to process
    await vi.waitFor(() => expect(onClose).toHaveBeenCalled());
    expect(onEvent).toHaveBeenCalledWith("message", { id: 1 });
  });

  it("calls onEvent with raw data when JSON parsing fails", async () => {
    const onEvent = vi.fn();
    const onClose = vi.fn();

    vi.stubGlobal(
      "fetch",
      mockFetchResponse(["data: not-json\n\n"]),
    );

    subscribeSSE("/v1/test", { onEvent, onClose });

    await vi.waitFor(() => expect(onClose).toHaveBeenCalled());
    expect(onEvent).toHaveBeenCalledWith("message", "not-json");
  });

  it("forwards named event types", async () => {
    const onEvent = vi.fn();
    const onClose = vi.fn();

    vi.stubGlobal(
      "fetch",
      mockFetchResponse(["event: run.status\ndata: {\"status\":\"running\"}\n\n"]),
    );

    subscribeSSE("/v1/test", { onEvent, onClose });

    await vi.waitFor(() => expect(onClose).toHaveBeenCalled());
    expect(onEvent).toHaveBeenCalledWith("run.status", {
      status: "running",
    });
  });

  it("calls onError when fetch returns non-ok status", async () => {
    const onError = vi.fn();
    const onClose = vi.fn();

    vi.stubGlobal("fetch", mockFetchResponse([], 500));

    subscribeSSE("/v1/test", { onEvent: vi.fn(), onError, onClose });

    await vi.waitFor(() => expect(onClose).toHaveBeenCalled());
    expect(onError).toHaveBeenCalledWith("SSE connection failed: 500");
  });

  it("calls onClose when stream ends", async () => {
    const onClose = vi.fn();

    vi.stubGlobal(
      "fetch",
      mockFetchResponse(["data: {\"done\":true}\n\n"]),
    );

    subscribeSSE("/v1/test", { onEvent: vi.fn(), onClose });

    await vi.waitFor(() => expect(onClose).toHaveBeenCalled());
  });

  it("handles abort without calling onError", async () => {
    const onError = vi.fn();

    // Use a stream that never ends until aborted
    const neverEndingFetch = vi.fn().mockResolvedValue({
      ok: true,
      status: 200,
      body: new ReadableStream({
        start() {
          // Never enqueue or close — waits forever until abort
        },
      }),
    } as unknown as Response);

    vi.stubGlobal("fetch", neverEndingFetch);

    const unsub = subscribeSSE("/v1/test", {
      onEvent: vi.fn(),
      onError,
    });

    // Abort the connection
    unsub();

    // Give the async handler time to process the abort
    await new Promise((r) => setTimeout(r, 50));
    expect(onError).not.toHaveBeenCalled();
  });
});
