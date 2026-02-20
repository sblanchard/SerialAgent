import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/vue";
import LoadingPanel from "../LoadingPanel.vue";

describe("LoadingPanel", () => {
  it("renders default 'Loading...' message", () => {
    render(LoadingPanel);
    expect(screen.getByText("Loading...")).toBeTruthy();
  });

  it("renders custom message", () => {
    render(LoadingPanel, { props: { message: "Fetching data..." } });
    expect(screen.getByText("Fetching data...")).toBeTruthy();
  });

  it("contains spinner element", () => {
    const { container } = render(LoadingPanel);
    expect(container.querySelector(".spinner")).toBeTruthy();
  });
});
