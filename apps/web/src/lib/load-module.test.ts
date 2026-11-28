import { NetworkError } from "@tokuly/core/errors";
import { afterEach, describe, expect, it, vi } from "vitest";
import { loadModule, loadNow } from "./load-module";

/** The browser's `online` event, on a stand-in window. */
const device = new EventTarget();
vi.stubGlobal("window", device);
const backOnline = () => device.dispatchEvent(new Event("online"));

/** A load whose outcome the test decides. */
function chunk() {
  const attempts: { resolve(module: string): void; reject(): void }[] = [];
  const load = () =>
    new Promise<string>((resolve, reject) => attempts.push({ resolve, reject: () => reject() }));
  return { load, attempts };
}

const settle = () => new Promise((resolve) => setTimeout(resolve));

describe("loading a chunk", () => {
  let stop = () => {};
  afterEach(() => stop());

  it("tries again when the device comes back online after a failed load", async () => {
    const { load, attempts } = chunk();
    const loaded = vi.fn();
    stop = loadModule(load, loaded);
    attempts[0]?.reject();
    await settle();
    backOnline();
    attempts[1]?.resolve("module");
    await settle();
    expect(loaded).toHaveBeenCalledWith("module");
  });

  it("tries again when the device came back online while the failing load was on its way", async () => {
    const { load, attempts } = chunk();
    const loaded = vi.fn();
    stop = loadModule(load, loaded);
    backOnline();
    expect(attempts).toHaveLength(1);
    attempts[0]?.reject();
    await settle();
    expect(attempts).toHaveLength(2);
    attempts[1]?.resolve("module");
    await settle();
    expect(loaded).toHaveBeenCalledOnce();
  });

  it("stops trying once stopped", async () => {
    const { load, attempts } = chunk();
    loadModule(load, vi.fn())();
    attempts[0]?.reject();
    await settle();
    backOnline();
    expect(attempts).toHaveLength(1);
  });
});

describe("loading a chunk something waits on", () => {
  it("rejects with a network error, and loads again on the next call", async () => {
    const { load, attempts } = chunk();
    const first = loadNow(load);
    attempts[0]?.reject();
    await expect(first).rejects.toBeInstanceOf(NetworkError);

    const second = loadNow(load);
    attempts[1]?.resolve("module");
    await expect(second).resolves.toBe("module");
  });
});
