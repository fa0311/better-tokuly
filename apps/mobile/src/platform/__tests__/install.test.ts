import { forgetPreviousInstall } from "../install";
import { readSecure, writeSecure } from "../secure-storage";

/** The app's own storage, at the edge: an uninstall empties it. */
const files = new Set<string>();
jest.mock("expo-file-system", () => ({
  Paths: { document: "documents" },
  File: class {
    private readonly path: string;
    constructor(directory: string, name: string) {
      this.path = `${directory}/${name}`;
    }
    get exists() {
      return mockFiles().has(this.path);
    }
    create() {
      mockFiles().add(this.path);
    }
  },
}));
const mockFiles = () => files;

const KEYS = ["tokuly.session.active", "tokuly.session.parked"];

beforeEach(async () => {
  files.clear();
  // What a previous install left in the keychain.
  await writeSecure("tokuly.session.active", "rt-previous");
  await writeSecure("tokuly.session.parked", '{"s":"rt-parked"}');
});

describe("forgetPreviousInstall", () => {
  it("forgets the accounts of a previous install on the first launch", async () => {
    await forgetPreviousInstall(KEYS);
    expect(await readSecure("tokuly.session.active")).toBeNull();
    expect(await readSecure("tokuly.session.parked")).toBeNull();
  });

  it("keeps them on every launch after that", async () => {
    await forgetPreviousInstall(KEYS);
    await writeSecure("tokuly.session.active", "rt-now");
    await forgetPreviousInstall(KEYS);
    expect(await readSecure("tokuly.session.active")).toBe("rt-now");
  });
});
