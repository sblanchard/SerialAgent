import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/vue";
import RunStatusBadge from "../RunStatusBadge.vue";

describe("RunStatusBadge", () => {
  it.each(["queued", "running", "completed", "failed", "stopped"] as const)(
    "renders %s status text",
    (status) => {
      render(RunStatusBadge, { props: { status } });
      expect(screen.getByText(status)).toBeTruthy();
    },
  );

  it("applies sm size class when size=sm", () => {
    const { container } = render(RunStatusBadge, {
      props: { status: "running", size: "sm" },
    });
    expect(container.querySelector(".sm")).toBeTruthy();
  });

  it("applies correct color class for completed", () => {
    const { container } = render(RunStatusBadge, {
      props: { status: "completed" },
    });
    expect(container.querySelector(".badge-green")).toBeTruthy();
  });

  it("applies pulse animation for running status", () => {
    const { container } = render(RunStatusBadge, {
      props: { status: "running" },
    });
    expect(container.querySelector(".pulse")).toBeTruthy();
  });
});
