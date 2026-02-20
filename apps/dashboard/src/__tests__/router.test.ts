import { describe, it, expect } from "vitest";
import { router } from "../router";

describe("router", () => {
  it("has 18 routes defined", () => {
    expect(router.getRoutes().length).toBe(18);
  });

  it("resolves /chat to the chat route", () => {
    const resolved = router.resolve("/chat");
    expect(resolved.name).toBe("chat");
  });

  it("resolves / to the overview route", () => {
    const resolved = router.resolve("/");
    expect(resolved.name).toBe("overview");
  });
});
