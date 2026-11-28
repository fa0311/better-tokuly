import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

/** Whether the strings chunk of `chat` arrives: the test decides. */
let chatArrives = false;

vi.mock(import("@tokuly/i18n/load"), async (original) => {
  const actual = await original();
  return {
    ...actual,
    loadNamespace: (language, namespace) =>
      namespace !== "chat" || chatArrives
        ? actual.loadNamespace(language, namespace)
        : Promise.reject(new TypeError("Failed to fetch dynamically imported module")),
  };
});

/** The browser's instance, as a page in Japanese starts it. */
async function browserI18n() {
  vi.resetModules();
  vi.stubGlobal("document", { documentElement: { lang: "ja" } });
  vi.stubGlobal("window", new EventTarget());
  const i18n = (await import("./i18n")).default;
  await vi.waitFor(() => expect(i18n.hasResourceBundle("ja", "common")).toBe(true));
  return i18n;
}

/** Until the chunk arrives, then once more, every lookup asks for `chat` (the app renders again). */
const lookUp = (i18n: Awaited<ReturnType<typeof browserI18n>>) => i18n.t("chat:actions.retry");

describe("strings whose chunk did not arrive", () => {
  beforeEach(() => {
    chatArrives = false;
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it("show nothing, and arrive when tried again a moment later", async () => {
    const i18n = await browserI18n();
    expect(lookUp(i18n)).toBe("");
    await vi.advanceTimersByTimeAsync(0);
    chatArrives = true;
    await vi.advanceTimersByTimeAsync(1_000);
    expect(lookUp(i18n)).toBe("もう一度");
  });

  it("load again on the next lookup after the last try, and once the device is back online", async () => {
    const i18n = await browserI18n();
    expect(lookUp(i18n)).toBe("");
    await vi.advanceTimersByTimeAsync(60_000);
    // Every try failed: still nothing shown, never the key.
    expect(lookUp(i18n)).toBe("");
    await vi.advanceTimersByTimeAsync(60_000);

    chatArrives = true;
    window.dispatchEvent(new Event("online"));
    await vi.advanceTimersByTimeAsync(0);
    expect(i18n.hasResourceBundle("ja", "chat")).toBe(true);
    expect(lookUp(i18n)).toBe("もう一度");
  });
});
