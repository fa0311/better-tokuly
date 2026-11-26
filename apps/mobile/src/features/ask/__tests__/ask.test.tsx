import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor } from "@testing-library/react-native";
import { ask, create } from "@tokuly/api";
import type { ReactNode } from "react";
import { Alert, type AlertButton } from "react-native";
import { SafeAreaProvider } from "react-native-safe-area-context";
import { api } from "#/lib/api";
import { screenFor } from "#/seams/links";
import { AskForm } from "../ask-form";
import { InboxScreen } from "../inbox-screen";
import links from "../links";

jest.mock("#/lib/api", () => ({ api: jest.fn(), apiWith: jest.fn(), authClient: jest.fn() }));
jest.mock("expo-router", () => ({ router: { push: jest.fn(), back: jest.fn() } }));
jest.mock("#/features/session/account", () => {
  const { api } = jest.requireMock("#/lib/api");
  return {
    currentAccount: () => ({ sessionId: "s1", userId: "me", api: api() }),
    isActive: () => true,
  };
});
jest.mock("#/features/session/store", () => ({
  useMe: () => ({ user: { id: "me", handle: "me" } }),
}));

const asker = { id: "u-secret", handle: "secret", displayName: "ひみつの人" };

/** The question-box API as the server answers it, with what the screen sent. */
function server(questions: ask.Question[]) {
  const sent: Record<string, unknown[]> = {};
  const note = (method: string, request: unknown) => {
    sent[method] = [...(sent[method] ?? []), request];
  };
  const record = (method: string) => async (request: unknown) => {
    note(method, request);
    return {};
  };
  jest.mocked(api).mockReturnValue({
    service: () => ({
      listInbox: async () => ({ questions, page: { nextCursor: "", hasMore: false } }),
      getInboxCounts: async () => ({ counts: { unread: 1, inbox: 1 } }),
      markQuestionsRead: async (request: unknown) => {
        note("markQuestionsRead", request);
        return { counts: { unread: 0, inbox: 1 } };
      },
      blockSender: record("blockSender"),
      askQuestion: record("askQuestion"),
    }),
  } as never);
  return sent;
}

async function show(children: ReactNode) {
  await render(
    <SafeAreaProvider
      initialMetrics={{
        frame: { x: 0, y: 0, width: 320, height: 640 },
        insets: { top: 0, left: 0, right: 0, bottom: 0 },
      }}
    >
      <QueryClientProvider client={new QueryClient()}>{children}</QueryClientProvider>
    </SafeAreaProvider>,
  );
}

describe("the inbox", () => {
  const anonymous = create(ask.QuestionSchema, {
    id: "q1",
    text: "好きな食べ物は?",
    anonymous: true,
    folder: ask.QuestionFolder.INBOX,
  });

  it("shows an anonymous question without a name, and reads it on sight", async () => {
    const sent = server([anonymous]);
    await show(<InboxScreen />);
    expect(await screen.findByText("好きな食べ物は?")).toBeTruthy();
    expect(screen.getByText("匿名さん")).toBeTruthy();
    await waitFor(() => expect(sent.markQuestionsRead).toEqual([{ questionIds: ["q1"] }]));
  });

  it("blocks whoever sent a question by the question, naming nobody", async () => {
    const sent = server([anonymous]);
    const alert = jest.spyOn(Alert, "alert").mockImplementation((_title, _body, buttons) => {
      (buttons as AlertButton[] | undefined)?.find((b) => b.style === "destructive")?.onPress?.();
    });
    await show(<InboxScreen />);
    await fireEvent.press(await screen.findByTestId("ask-question-menu"));
    await fireEvent.press(screen.getByTestId("sheet-ask-block"));
    await waitFor(() =>
      expect(sent.blockSender).toEqual([{ target: { case: "questionId", value: "q1" } }]),
    );
    alert.mockRestore();
  });

  it("shows a signed question's sender", async () => {
    server([
      create(ask.QuestionSchema, {
        id: "q2",
        text: "元気?",
        anonymous: false,
        asker,
        folder: ask.QuestionFolder.INBOX,
      }),
    ]);
    await show(<InboxScreen />);
    expect(await screen.findByText("ひみつの人")).toBeTruthy();
  });
});

describe("asking", () => {
  const box = (fields: { anonymousAvailable: boolean; allowSigned: boolean }) =>
    create(ask.QuestionBoxSchema, {
      owner: { id: "u1", handle: "alice", displayName: "アリス" },
      open: true,
      canAsk: true,
      ...fields,
    });

  it("is anonymous unless the asker signs", async () => {
    const sent = server([]);
    await show(<AskForm box={box({ anonymousAvailable: true, allowSigned: true })} />);
    await fireEvent.changeText(screen.getByTestId("ask-input"), "  おすすめの本は?  ");
    await fireEvent.press(screen.getByTestId("ask-send"));
    expect(await screen.findByTestId("ask-sent")).toBeTruthy();
    expect(sent.askQuestion).toEqual([
      { user: "u1", text: "おすすめの本は?", signed: false, imageAssetId: "" },
    ]);

    await fireEvent.press(screen.getByTestId("ask-another"));
    await fireEvent.press(screen.getByTestId("ask-signed"));
    await fireEvent.changeText(screen.getByTestId("ask-input"), "元気?");
    await fireEvent.press(screen.getByTestId("ask-send"));
    await waitFor(() => expect(sent.askQuestion).toHaveLength(2));
    expect(sent.askQuestion?.[1]).toMatchObject({ signed: true });
  });

  it("says so when the box is too small to hide an asker in", async () => {
    const sent = server([]);
    await show(<AskForm box={box({ anonymousAvailable: false, allowSigned: true })} />);
    expect(screen.queryByTestId("ask-anonymous")).toBeNull();
    expect(screen.getByText(/名前つきでだけ送れるよ/)).toBeTruthy();
    await fireEvent.changeText(screen.getByTestId("ask-input"), "元気?");
    await fireEvent.press(screen.getByTestId("ask-send"));
    await waitFor(() => expect(sent.askQuestion?.[0]).toMatchObject({ signed: true }));
  });
});

describe("links", () => {
  it("open the box's screens here and prompts, polls and flames on the web", () => {
    expect(screenFor("/ask/inbox", links)).toBe("/ask/inbox");
    expect(screenFor("/ask/q/q1", links)).toBe("/ask/q/q1");
    expect(screenFor("/q/alice?a=q1", links)).toBe("/ask/u/alice?a=q1");
    expect(screenFor("/u/alice/ask", links)).toBe("/ask/u/alice");
    expect(screenFor("/ask/prompts/p1", links)).toBe(
      `/web?path=${encodeURIComponent("/ask/prompts/p1")}`,
    );
  });
});
