import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor } from "@testing-library/react-native";
import { create, search } from "@tokuly/api";
import { router } from "expo-router";
import { SafeAreaProvider } from "react-native-safe-area-context";
import { api } from "#/lib/api";
import { SearchScreen, type SearchStart } from "../search-screen";

jest.mock("#/lib/api", () => ({ api: jest.fn(), apiWith: jest.fn(), authClient: jest.fn() }));
jest.mock("expo-router", () => ({ router: { push: jest.fn() } }));

const K = search.SearchKind;

const hit = (kind: search.SearchKind, id: string, title: string, fields: object = {}) =>
  create(search.SearchHitSchema, { kind, id, title, extraJson: "{}", ...fields });

/** The search API as the server answers it, with what the screen asked. */
function server() {
  const asked: { query: string; kind: search.SearchKind; skipHistory: boolean }[] = [];
  const forgotten: string[] = [];
  let recent = [
    create(search.RecentSearchSchema, { id: "r1", query: "カフェ" }),
    create(search.RecentSearchSchema, { id: "r2", query: "文化祭" }),
  ];
  jest.mocked(api).mockReturnValue({
    service: () => ({
      listRecentSearches: async () => ({ searches: recent, historyPaused: false }),
      getTrending: async () => ({ queries: [{ query: "テスト", rank: 1 }] }),
      deleteRecentSearch: async ({ id }: { id: string }) => {
        forgotten.push(id);
        recent = recent.filter((item) => item.id !== id);
        return {};
      },
      suggest: async () => ({ completions: [], hits: [] }),
      search: async (request: { query: string; kind: search.SearchKind; skipHistory: boolean }) => {
        asked.push(request);
        const hits =
          request.kind === K.NOTE
            ? [hit(K.NOTE, "n1", "さくらのノート")]
            : [
                hit(K.USER, "u1", "さくら", { extraJson: '{"handle":"sakura"}' }),
                hit(K.HASHTAG, "h1", "#さくら", { extraJson: '{"tag":"さくら"}' }),
              ];
        return {
          hits,
          page: { nextCursor: "" },
          counts: [
            { kind: K.USER, count: 1n },
            { kind: K.NOTE, count: 1n },
            { kind: K.HASHTAG, count: 1n },
          ],
        };
      },
    }),
  } as never);
  return { asked, forgotten };
}

async function show(start: SearchStart = {}) {
  await render(
    <SafeAreaProvider
      initialMetrics={{
        frame: { x: 0, y: 0, width: 320, height: 640 },
        insets: { top: 0, left: 0, right: 0, bottom: 0 },
      }}
    >
      <QueryClientProvider client={new QueryClient()}>
        <SearchScreen start={start} />
      </QueryClientProvider>
    </SafeAreaProvider>,
  );
}

describe("SearchScreen", () => {
  it("shows recent searches and trends, and forgets one", async () => {
    const { forgotten } = server();
    await show();
    expect(await screen.findByText("カフェ")).toBeOnTheScreen();
    expect(await screen.findByText("テスト")).toBeOnTheScreen();
    await fireEvent.press(screen.getByTestId("search-recent:r1-forget"));
    await waitFor(() => expect(screen.queryByText("カフェ")).not.toBeOnTheScreen());
    expect(forgotten).toEqual(["r1"]);
  });

  it("remembers what was typed, opens people and searches a tag", async () => {
    const { asked } = server();
    await show();
    await fireEvent.changeText(screen.getByTestId("search-input"), "さくら");
    await fireEvent(screen.getByTestId("search-input"), "submitEditing");
    expect(await screen.findByText("さくら")).toBeOnTheScreen();
    expect(asked[0]).toMatchObject({ query: "さくら", kind: K.UNSPECIFIED, skipHistory: false });

    // Only the kinds with matches get a tab.
    await fireEvent.press(screen.getByTestId("search-kind-notes"));
    expect(await screen.findByText("さくらのノート")).toBeOnTheScreen();
    expect(screen.queryByTestId("search-kind-videos")).not.toBeOnTheScreen();
    await fireEvent.press(screen.getByTestId("search-kind-all"));

    await fireEvent.press(await screen.findByTestId("search-hit-u1"));
    expect(router.push).toHaveBeenCalledWith("/u/sakura");

    await fireEvent.press(screen.getByTestId("search-hit-h1"));
    await waitFor(() =>
      expect(asked.at(-1)).toMatchObject({ query: "#さくら", skipHistory: false }),
    );
  });

  it("does not remember a search opened from a link", async () => {
    const { asked } = server();
    await show({ query: "さくら", kind: K.NOTE });
    expect(await screen.findByText("さくらのノート")).toBeOnTheScreen();
    expect(asked.every((request) => request.skipHistory)).toBe(true);
  });
});
