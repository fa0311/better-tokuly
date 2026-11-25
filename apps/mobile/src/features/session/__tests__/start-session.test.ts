import { api } from "#/lib/api";
import { writeSecure } from "#/platform/secure-storage";
import { startSession, useSessionStore } from "../session";

/** The app's own storage, at the edge: this launch cannot write to it. */
jest.mock("expo-file-system", () => ({
  Paths: { document: "documents" },
  File: class {
    get exists() {
      return false;
    }
    create() {
      throw new Error("disk full");
    }
  },
}));
/** The server, at the edge: nothing may reach it without a known install. */
const calls: string[] = [];
jest.mock("expo/fetch", () => ({
  fetch: async (input: string) => {
    calls.push(new URL(input).pathname);
    throw new Error("offline");
  },
}));

describe("startSession", () => {
  it("starts signed out, and says why, when the install cannot be checked", async () => {
    await writeSecure("tokuly.session.active", "rt-previous");
    const warn = jest.spyOn(console, "warn").mockImplementation(() => undefined);

    await expect(startSession()).resolves.toBeUndefined();

    expect(useSessionStore.getState().state.status).toBe("signedOut");
    expect(api().session.state.status).toBe("signedOut");
    expect(calls).toEqual([]);
    expect(warn).toHaveBeenCalledWith(expect.stringContaining("signed out"), expect.any(Error));
    warn.mockRestore();
  });
});
