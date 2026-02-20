import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/vue";
import NavBar from "../NavBar.vue";

// Stub vue-router
vi.mock("vue-router", () => ({
  useRoute: () => ({ path: "/" }),
  RouterLink: {
    template: '<a class="router-link"><slot /></a>',
    props: ["to"],
  },
}));

// Stub matchMedia for useTheme
beforeEach(() => {
  vi.stubGlobal("matchMedia", vi.fn().mockReturnValue({ matches: false }));
  localStorage.clear();
});

describe("NavBar", () => {
  it("renders the SA logo", () => {
    render(NavBar);
    expect(screen.getByText("SA")).toBeTruthy();
  });

  it("renders all 15 navigation links", () => {
    const { container } = render(NavBar);
    const links = container.querySelectorAll("li");
    expect(links.length).toBe(15);
  });

  it("renders theme toggle button", () => {
    render(NavBar);
    expect(screen.getByText(/Mode/)).toBeTruthy();
  });
});
