import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { subscribeSSE } from "../sse";
import type { SseOptions } from "../sse";

// ── Mock EventSource ────────────────────────────────────────────────

type Listener = (ev: MessageEvent) => void;

class MockEventSource {
  static readonly CLOSED = 2;
  readonly CLOSED = 2;

  readyState = 0;
  url: string;
  onmessage: ((ev: MessageEvent) => void) | null = null;
  onerror: ((ev: Event) => void) | null = null;

  private listeners = new Map<string, Listener[]>();
  closed = false;

  constructor(url: string) {
    this.url = url;
  }

  addEventListener(type: string, handler: Listener) {
    const existing = this.listeners.get(type) ?? [];
    this.listeners.set(type, [...existing, handler]);
  }

  close() {
    this.closed = true;
  }

  // Test helpers
  simulateMessage(data: string) {
    this.onmessage?.({ data } as MessageEvent);
  }

  simulateNamedEvent(type: string, data: string) {
    const handlers = this.listeners.get(type) ?? [];
    for (const handler of handlers) {
      handler({ data } as MessageEvent);
    }
  }

  simulateError() {
    this.onerror?.({} as Event);
  }

  simulateClose() {
    this.readyState = MockEventSource.CLOSED;
    this.onerror?.({} as Event);
  }
}

let lastInstance: MockEventSource | null = null;

beforeEach(() => {
  lastInstance = null;
  vi.stubGlobal("EventSource", class extends MockEventSource {
    constructor(url: string) {
      super(url);
      lastInstance = this;
    }
  });
});

afterEach(() => {
  vi.unstubAllGlobals();
});

// ── Tests ───────────────────────────────────────────────────────────

describe("subscribeSSE", () => {
  it("returns a cleanup function that closes the EventSource", () => {
    const opts: SseOptions<unknown> = { onEvent: vi.fn() };
    const unsub = subscribeSSE("/v1/test", opts);

    expect(lastInstance).not.toBeNull();
    expect(lastInstance!.closed).toBe(false);

    unsub();
    expect(lastInstance!.closed).toBe(true);
  });

  it("calls onEvent with parsed JSON for generic messages", () => {
    const onEvent = vi.fn();
    subscribeSSE("/v1/test", { onEvent });

    lastInstance!.simulateMessage(JSON.stringify({ id: 1 }));
    expect(onEvent).toHaveBeenCalledWith("message", { id: 1 });
  });

  it("calls onEvent with raw data when JSON parsing fails", () => {
    const onEvent = vi.fn();
    subscribeSSE("/v1/test", { onEvent });

    lastInstance!.simulateMessage("not-json");
    expect(onEvent).toHaveBeenCalledWith("message", "not-json");
  });

  it("forwards named event types", () => {
    const onEvent = vi.fn();
    subscribeSSE("/v1/test", { onEvent });

    lastInstance!.simulateNamedEvent(
      "run.status",
      JSON.stringify({ status: "running" }),
    );
    expect(onEvent).toHaveBeenCalledWith("run.status", { status: "running" });
  });

  it("calls onError when EventSource emits error and is not closed", () => {
    const onError = vi.fn();
    subscribeSSE("/v1/test", { onEvent: vi.fn(), onError });

    lastInstance!.simulateError();
    expect(onError).toHaveBeenCalled();
  });

  it("calls onClose when EventSource readyState is CLOSED", () => {
    const onClose = vi.fn();
    subscribeSSE("/v1/test", { onEvent: vi.fn(), onClose });

    lastInstance!.simulateClose();
    expect(onClose).toHaveBeenCalled();
  });
});
