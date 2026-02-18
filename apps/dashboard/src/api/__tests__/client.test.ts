import { describe, it, expect, beforeEach } from "vitest";
import { buildHeaders, setApiToken, getApiToken } from "../client";

describe("buildHeaders", () => {
  beforeEach(() => {
    // Reset token state before each test by setting a known value then clearing
    // We use the public setter; internally apiToken is module-level state.
    setApiToken("");
  });

  it("returns an empty object when no token and no content type", () => {
    // Clear the token by setting empty string — buildHeaders guards on truthiness
    setApiToken("");
    const headers = buildHeaders();
    // Empty string is falsy so Authorization should not be present
    expect(headers).not.toHaveProperty("Authorization");
    expect(headers).not.toHaveProperty("Content-Type");
  });

  it("includes Content-Type when provided", () => {
    setApiToken("");
    const headers = buildHeaders("application/json");
    expect(headers["Content-Type"]).toBe("application/json");
    expect(headers).not.toHaveProperty("Authorization");
  });

  it("includes Authorization header when token is set", () => {
    setApiToken("test-token-abc");
    const headers = buildHeaders();
    expect(headers["Authorization"]).toBe("Bearer test-token-abc");
  });

  it("includes both Authorization and Content-Type when both are present", () => {
    setApiToken("my-secret-token");
    const headers = buildHeaders("application/json");
    expect(headers["Authorization"]).toBe("Bearer my-secret-token");
    expect(headers["Content-Type"]).toBe("application/json");
  });
});

describe("token management", () => {
  beforeEach(() => {
    setApiToken("");
  });

  it("getApiToken returns null initially or empty after reset", () => {
    // After setting empty string, getApiToken returns ""
    const token = getApiToken();
    expect(token).toBe("");
  });

  it("setApiToken / getApiToken round-trips correctly", () => {
    setApiToken("round-trip-token");
    expect(getApiToken()).toBe("round-trip-token");
  });
});
