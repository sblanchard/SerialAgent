import { describe, it, expect } from "vitest";
import { render, screen, fireEvent } from "@testing-library/vue";
import ThoughtBubble from "../ThoughtBubble.vue";

describe("ThoughtBubble", () => {
  const baseProps = { content: "Let me think about this...", timestamp: "12:00:00" };

  it("renders thought content", () => {
    render(ThoughtBubble, { props: baseProps });
    expect(screen.getByText("Let me think about this...")).toBeTruthy();
  });

  it("displays timestamp", () => {
    render(ThoughtBubble, { props: baseProps });
    expect(screen.getByText("12:00:00")).toBeTruthy();
  });

  it("starts collapsed by default", () => {
    const { container } = render(ThoughtBubble, { props: baseProps });
    expect(container.querySelector(".collapsed")).toBeTruthy();
  });

  it("expands on header click", async () => {
    const { container } = render(ThoughtBubble, { props: baseProps });
    const header = container.querySelector(".msg-header")!;
    await fireEvent.click(header);
    expect(container.querySelector(".collapsed")).toBeNull();
  });
});
