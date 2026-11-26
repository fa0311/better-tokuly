import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react-native";
import { common, create, notes } from "@tokuly/api";
import { Alert } from "react-native";
import { SafeAreaProvider } from "react-native-safe-area-context";
import { api } from "#/lib/api";
import { ComposeScreen } from "../compose-screen";

jest.mock("#/lib/api", () => ({ api: jest.fn(), apiWith: jest.fn(), authClient: jest.fn() }));
const mockNavigation = { setOptions: jest.fn(), dispatch: jest.fn() };
let mockLeaveGuard: ((options: { data: { action: unknown } }) => void) | undefined;
jest.mock("expo-router", () => ({
  router: { back: jest.fn(), push: jest.fn() },
  useNavigation: () => mockNavigation,
  usePreventRemove: (prevent: boolean, callback: typeof mockLeaveGuard) => {
    mockLeaveGuard = prevent ? callback : undefined;
  },
}));
jest.mock("#/features/media/pick", () => ({ pickFromLibrary: jest.fn() }));
jest.mock("expo-crypto", () => ({ randomUUID: () => "idempotency-1" }));

const FRIEND = create(common.UserSummarySchema, { id: "u-bob", handle: "bob", displayName: "Bob" });

/** The API of one account: what each notes and social call was asked, answered the way a server would. */
function account(options: { agreed?: boolean; draft?: notes.Draft } = {}) {
  const calls: { method: string; request: Record<string, unknown> }[] = [];
  let agreed = options.agreed ?? true;
  const record =
    (method: string, answer: (request: Record<string, unknown>) => unknown = () => ({})) =>
    async (request: Record<string, unknown>) => {
      calls.push({ method, request });
      return answer(request);
    };
  const services: Record<string, Record<string, unknown>> = {
    "tokuly.notes.v1.NoteService": {
      getNoteRules: async () => ({
        version: 3,
        agreed,
        agreedVersion: agreed ? 3 : 0,
        rules: [{ title: "やさしく", body: "みんなが気持ちよく使えるように" }],
      }),
      agreeToNoteRules: record("agreeToNoteRules", () => {
        agreed = true;
        return {};
      }),
      createNote: record("createNote", (request) => ({
        note: create(notes.NoteSchema, { id: "n-new", text: request.text as string }),
      })),
      searchHashtags: async () => ({ hashtags: [] }),
    },
    "tokuly.notes.v1.NoteDraftService": {
      saveDraft: record("saveDraft", () => ({})),
      getDraft: async () => ({ draft: options.draft }),
    },
    "tokuly.identity.v1.AccountService": {
      getSettings: async () => ({
        settings: { privacy: { defaultNoteVisibility: common.Visibility.FRIENDS } },
      }),
    },
    "tokuly.social.v1.CircleService": { listCircles: async () => ({ circles: [] }) },
    "tokuly.social.v1.FriendService": {
      listFriends: async () => ({ friends: [{ user: FRIEND }] }),
    },
  };
  const object = {
    service: (descriptor: { typeName: string }) => services[descriptor.typeName] ?? {},
  };
  return { object, calls };
}

async function open(element: React.ReactElement) {
  await render(
    <SafeAreaProvider
      initialMetrics={{
        frame: { x: 0, y: 0, width: 320, height: 640 },
        insets: { top: 0, left: 0, right: 0, bottom: 0 },
      }}
    >
      <QueryClientProvider client={new QueryClient()}>{element}</QueryClientProvider>
    </SafeAreaProvider>,
  );
}

beforeEach(() => {
  mockLeaveGuard = undefined;
  jest.spyOn(Alert, "alert").mockImplementation(() => {});
});

describe("ComposeScreen", () => {
  it("asks for the rules first, then lets the account write", async () => {
    const alice = account({ agreed: false });
    jest.mocked(api).mockReturnValue(alice.object as never);
    await open(<ComposeScreen />);
    await fireEvent.press(await screen.findByTestId("note-rules-agree"));
    expect(await screen.findByTestId("note-text")).toBeOnTheScreen();
    expect(alice.calls).toContainEqual({ method: "agreeToNoteRules", request: { version: 3 } });
  });

  it("posts to the people picked, once, with the client of the account that tapped", async () => {
    const alice = account();
    const bob = account();
    jest.mocked(api).mockReturnValue(alice.object as never);
    await open(<ComposeScreen />);
    await fireEvent.changeText(await screen.findByTestId("note-text"), "ないしょの話 #himitsu");
    await fireEvent.press(screen.getByTestId("note-audience"));
    await fireEvent.press(await screen.findByTestId("audience-direct"));
    await fireEvent.press(await screen.findByTestId("audience-person-bob"));
    await fireEvent.press(screen.getByTestId("audience-done"));
    // Another account takes over while the note is on its way: it still goes out as the first.
    await fireEvent.press(screen.getByTestId("note-post"));
    jest.mocked(api).mockReturnValue(bob.object as never);
    await waitFor(() =>
      expect(alice.calls.some((call) => call.method === "createNote")).toBe(true),
    );
    const sent = alice.calls.find((call) => call.method === "createNote")!.request;
    expect(sent).toMatchObject({
      text: "ないしょの話 #himitsu",
      visibility: common.Visibility.PRIVATE,
      directUserIds: ["u-bob"],
    });
    expect(sent.idempotencyKey).toBe("idempotency-1");
    expect(bob.calls).toEqual([]);
  });

  it("offers to keep what was written as a draft when leaving", async () => {
    const alice = account();
    jest.mocked(api).mockReturnValue(alice.object as never);
    await open(<ComposeScreen />);
    await fireEvent.changeText(await screen.findByTestId("note-text"), "あとで書く");
    expect(mockLeaveGuard).toBeDefined();
    await act(async () => mockLeaveGuard?.({ data: { action: { type: "GO_BACK" } } }));
    const buttons = jest.mocked(Alert.alert).mock.calls.at(-1)?.[2] ?? [];
    await act(async () => buttons.find((button) => button.text === "下書き")?.onPress?.());
    await waitFor(() =>
      expect(alice.calls).toContainEqual({
        method: "saveDraft",
        request: expect.objectContaining({
          draftId: "",
          content: expect.objectContaining({ text: "あとで書く" }),
        }),
      }),
    );
  });

  it("says who the note goes to when nothing is picked: the account's usual audience", async () => {
    const alice = account();
    jest.mocked(api).mockReturnValue(alice.object as never);
    await open(<ComposeScreen />);
    expect(await screen.findByText("ともだち ▾")).toBeOnTheScreen();
    await fireEvent.changeText(screen.getByTestId("note-text"), "いつもどおり");
    await fireEvent.press(screen.getByTestId("note-post"));
    await waitFor(() =>
      expect(alice.calls.some((call) => call.method === "createNote")).toBe(true),
    );
    // The choice stays the server's: it knows the setting as it is now.
    expect(alice.calls.find((call) => call.method === "createNote")!.request).toMatchObject({
      visibility: common.Visibility.UNSPECIFIED,
    });
  });

  it("keeps the time a scheduled draft goes out when it is saved again", async () => {
    const scheduledAt = { seconds: 1_900_000_000n, nanos: 0 };
    const alice = account({
      draft: create(notes.DraftSchema, {
        id: "d-1",
        content: { text: "あしたの朝に" },
        scheduledAt,
      }),
    });
    jest.mocked(api).mockReturnValue(alice.object as never);
    await open(<ComposeScreen draft="d-1" />);
    await fireEvent.changeText(await screen.findByTestId("note-text"), "あしたの朝に！");
    await fireEvent.press(screen.getByTestId("note-save-draft"));
    await waitFor(() =>
      expect(alice.calls).toContainEqual({
        method: "saveDraft",
        request: expect.objectContaining({
          draftId: "d-1",
          scheduledAt: expect.objectContaining({ seconds: 1_900_000_000n }),
        }),
      }),
    );
  });
});
