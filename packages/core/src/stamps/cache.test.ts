import { QueryClient } from "@tanstack/query-core";
import { create, stamp } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import { markUninstalled, patchStamp } from "./cache";
import { stampKeys } from "./keys";

const item = (id: string) =>
  create(stamp.StampSchema, { stampRef: { id, name: id }, favorited: false });

describe("patchStamp", () => {
  it("changes the stamp wherever it is cached and leaves the rest untouched", () => {
    const queryClient = new QueryClient();
    const library = create(stamp.GetLibraryResponseSchema, {
      recents: [item("a"), item("b")],
      favorites: [],
    });
    queryClient.setQueryData(stampKeys.libraryHome, library);
    queryClient.setQueryData(stampKeys.personal, [item("a")]);

    patchStamp(queryClient, "a", { favorited: true });

    const home = queryClient.getQueryData<stamp.GetLibraryResponse>(stampKeys.libraryHome)!;
    expect(home.recents[0]?.favorited).toBe(true);
    expect(home.recents[1]).toBe(library.recents[1]);
    expect(queryClient.getQueryData<stamp.Stamp[]>(stampKeys.personal)?.[0]?.favorited).toBe(true);
  });
});

describe("markUninstalled", () => {
  it("clears the installed mark of the pack's cards", () => {
    const queryClient = new QueryClient();
    queryClient.setQueryData(
      stampKeys.storeHome,
      create(stamp.GetStoreHomeResponseSchema, {
        featured: [{ id: "p", viewer: { installed: true, owned: true } }, { id: "q" }],
      }),
    );

    markUninstalled(queryClient, "p");

    const home = queryClient.getQueryData<stamp.GetStoreHomeResponse>(stampKeys.storeHome)!;
    expect(home.featured[0]?.viewer).toMatchObject({ installed: false, owned: true });
    expect(home.featured[1]?.viewer).toBeUndefined();
  });
});
