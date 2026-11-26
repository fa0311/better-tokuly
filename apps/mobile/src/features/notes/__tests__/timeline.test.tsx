import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react-native";
import { create, notes } from "@tokuly/api";
import { SafeAreaProvider } from "react-native-safe-area-context";
import { api } from "#/lib/api";
import { markNoteDeleted } from "../data";
import { hintTimelines } from "../live";
import { Timeline } from "../timeline";

jest.mock("#/lib/api", () => ({ api: jest.fn(), apiWith: jest.fn(), authClient: jest.fn() }));
jest.mock("expo-router", () => ({ router: { push: jest.fn() } }));
// The connection is the edge: cards follow their note's topic on it.
jest.mock("#/lib/realtime", () => ({
  realtime: () => ({ subscribe: () => ({ release: () => {} }), start: () => {} }),
}));

const entry = (id: string, text: string) =>
  create(notes.TimelineEntrySchema, {
    note: create(notes.NoteSchema, {
      id,
      text,
      author: { id: `author-${id}`, handle: "alice", displayName: "Alice" },
    }),
  });

describe("Timeline", () => {
  it("keeps what is on screen until asked, then puts the new notes on top", async () => {
    let home = [entry("a", "さいしょのノート")];
    jest.mocked(api).mockReturnValue({
      service: () => ({
        getHomeTimeline: async () => ({ entries: home, page: { nextCursor: "" } }),
        recordNoteViews: async () => ({}),
      }),
    } as never);
    const queryClient = new QueryClient();
    await render(
      <SafeAreaProvider
        initialMetrics={{
          frame: { x: 0, y: 0, width: 320, height: 640 },
          insets: { top: 0, left: 0, right: 0, bottom: 0 },
        }}
      >
        <QueryClientProvider client={queryClient}>
          <Timeline source={{ kind: "home" }} empty={{ emoji: "📝", title: "まだないよ" }} />
        </QueryClientProvider>
      </SafeAreaProvider>,
    );
    expect(await screen.findByText("さいしょのノート")).toBeOnTheScreen();

    // A friend posted: the hint reads the newest page, and a pill says so.
    home = [entry("b", "あたらしいノート"), ...home];
    await act(async () => hintTimelines());
    const pill = await screen.findByTestId("timeline-new");
    expect(screen.getByText("新しいノート 1件")).toBeOnTheScreen();
    expect(screen.queryByText("あたらしいノート")).not.toBeOnTheScreen();
    await fireEvent.press(pill);
    expect(await screen.findByText("あたらしいノート")).toBeOnTheScreen();

    // A note deleted elsewhere leaves the list without the pages being read again.
    await act(async () => markNoteDeleted(queryClient, "a"));
    await waitFor(() => expect(screen.queryByText("さいしょのノート")).not.toBeOnTheScreen());
  });
});
