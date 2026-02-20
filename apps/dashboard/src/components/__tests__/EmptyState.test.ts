import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/vue";
import EmptyState from "../EmptyState.vue";

describe("EmptyState", () => {
  it("renders title", () => {
    render(EmptyState, { props: { title: "Nothing here" } });
    expect(screen.getByText("Nothing here")).toBeTruthy();
  });

  it("renders description when provided", () => {
    render(EmptyState, {
      props: { title: "Empty", description: "No items found" },
    });
    expect(screen.getByText("No items found")).toBeTruthy();
  });

  it("renders icon when provided", () => {
    render(EmptyState, { props: { title: "Empty", icon: "?" } });
    expect(screen.getByText("?")).toBeTruthy();
  });

  it("renders slot content", () => {
    render(EmptyState, {
      props: { title: "Empty" },
      slots: { default: "Action content" },
    });
    expect(screen.getByText("Action content")).toBeTruthy();
  });
});
