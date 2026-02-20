import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/vue";
import Card from "../Card.vue";

describe("Card", () => {
  it("renders title when provided", () => {
    render(Card, { props: { title: "Test Title" } });
    expect(screen.getByText("Test Title")).toBeTruthy();
  });

  it("omits title element when no title prop", () => {
    const { container } = render(Card);
    expect(container.querySelector(".card-title")).toBeNull();
  });

  it("renders slot content", () => {
    render(Card, { slots: { default: "Slot content here" } });
    expect(screen.getByText("Slot content here")).toBeTruthy();
  });
});
