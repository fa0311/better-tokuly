import { act, renderHook } from "@testing-library/react-native";
import { pickFromLibrary } from "#/features/media/pick";
import { uploadFile } from "#/features/media/upload";
import type { Account } from "#/features/session/account";
import { useNotePhotos } from "../photos";

let mockActive: Account | null = null;
jest.mock("#/features/session/account", () => ({
  currentAccount: () => mockActive,
  isActive: (account: Account) => mockActive === account,
}));
jest.mock("#/features/media/pick", () => ({ pickFromLibrary: jest.fn() }));
jest.mock("#/features/media/upload", () => ({ uploadFile: jest.fn() }));
jest.mock("#/platform/transfer", () => ({
  keepFile: async (_key: string, uri: string) => ({ uri, size: 1 }),
  forgetFiles: jest.fn(),
}));

const account = (name: string) => ({ sessionId: name, userId: name, api: {} }) as Account;
const PHOTO = { uri: "file:///photo.jpg", name: "photo.jpg", contentType: "image/jpeg" };

describe("the photos of a note being written", () => {
  beforeEach(() => jest.mocked(uploadFile).mockReset());

  it("go up as the account that opened the library", async () => {
    const main = account("main");
    mockActive = main;
    jest.mocked(pickFromLibrary).mockResolvedValue({ media: [PHOTO] } as never);
    jest.mocked(uploadFile).mockResolvedValue({ id: "asset-1" } as never);
    const { result } = await renderHook(() => useNotePhotos());

    await act(() => result.current.pick());
    expect(jest.mocked(uploadFile).mock.calls[0]?.[0]).toBe(main);
    expect(result.current.assetIds).toEqual(["asset-1"]);
  });

  it("are not uploaded as whoever took over while the library was open", async () => {
    const main = account("main");
    mockActive = main;
    jest.mocked(pickFromLibrary).mockImplementation(async () => {
      // Another account takes over before the picture is chosen.
      mockActive = account("ura");
      return { media: [PHOTO] } as never;
    });
    const { result } = await renderHook(() => useNotePhotos());

    await act(() => result.current.pick());
    expect(uploadFile).not.toHaveBeenCalled();
    expect(result.current.photos).toEqual([]);
  });
});
