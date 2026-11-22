import { beforeEach, describe, expect, it, vi } from "vitest";
import { clearDrafts, loadDraft, saveDraft } from "./drafts";

/** A browser's storage, as far as drafts use it. */
function storage(): Storage {
  const values = new Map<string, string>();
  return {
    get length() {
      return values.size;
    },
    key: (index: number) => [...values.keys()][index] ?? null,
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => void values.set(key, value),
    removeItem: (key: string) => void values.delete(key),
    clear: () => values.clear(),
  };
}

beforeEach(() => vi.stubGlobal("window", { localStorage: storage() }));

describe("drafts", () => {
  it("belong to the person who typed them", () => {
    saveDraft("aoi", "group", "あとで送る");
    expect(loadDraft("aoi", "group")).toBe("あとで送る");
    // Someone else in the same conversation, on the same browser, starts with an empty composer.
    expect(loadDraft("ren", "group")).toBe("");
  });

  it("go away when the text does", () => {
    saveDraft("aoi", "group", "x");
    saveDraft("aoi", "group", "  ");
    expect(loadDraft("aoi", "group")).toBe("");
    expect(window.localStorage.length).toBe(0);
  });

  it("are all removed on sign-out, and nothing else is", () => {
    saveDraft("aoi", "group", "one");
    saveDraft("aoi", "direct", "two");
    window.localStorage.setItem("tokuly.theme", "dark");
    clearDrafts();
    expect(loadDraft("aoi", "group")).toBe("");
    expect(loadDraft("aoi", "direct")).toBe("");
    expect(window.localStorage.getItem("tokuly.theme")).toBe("dark");
  });
});
