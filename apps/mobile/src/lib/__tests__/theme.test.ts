import { colors, radius } from "@tokuly/core/tokens";

/** The Tailwind names of the app (tailwind.config.js reads no TypeScript, so it lists them). */
const config = require("../../../tailwind.config.js") as {
  theme: { extend: { colors: Record<string, string>; borderRadius: Record<string, string> } };
};

describe("the phone's Tailwind names", () => {
  it("are the shared tokens: every colour but the shadow's, and the radii", () => {
    const names = Object.keys(colors.light).filter((name) => name !== "shadow");
    expect(Object.keys(config.theme.extend.colors)).toEqual(names);
    expect(config.theme.extend.borderRadius).toEqual({
      card: `${radius.card}px`,
      control: `${radius.control}px`,
    });
  });
});
