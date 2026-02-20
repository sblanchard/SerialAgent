import { describe, it, expect } from "vitest";
import { render } from "@testing-library/vue";
import StatusDot from "../StatusDot.vue";

describe("StatusDot", () => {
  it.each(["ok", "warn", "error", "off"] as const)(
    "renders with %s status class",
    (status) => {
      const { container } = render(StatusDot, { props: { status } });
      const dot = container.querySelector(".dot");
      expect(dot).toBeTruthy();
      expect(dot!.classList.contains(status)).toBe(true);
    },
  );
});
