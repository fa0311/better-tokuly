import { readSecure } from "#/platform/secure-storage";
import { loadSequence, takeSequence } from "../sequence";

const settle = (ms = 20) => new Promise((resolve) => setTimeout(resolve, ms));

describe("the running count", () => {
  it("is an account's own: another account here does not go on from it when the clock went back", async () => {
    const now = Date.now();
    await loadSequence("alice");
    await loadSequence("bob");
    const alices = takeSequence("alice");
    const clock = jest.spyOn(Date, "now").mockReturnValue(now - 3_600_000);
    try {
      // bob's numbers follow the clock, as on a device alice never used.
      expect(takeSequence("bob")).toBe(BigInt(now - 3_600_000));
      expect(takeSequence("alice")).toBe(alices + 1n);
    } finally {
      clock.mockRestore();
    }
    await settle();
    expect(await readSecure("tokuly.map.sequence.alice")).toBe(String(alices + 1n));
    expect(await readSecure("tokuly.map.sequence.bob")).toBe(String(now - 3_600_000));
  });
});
