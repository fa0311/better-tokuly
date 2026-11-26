import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor } from "@testing-library/react-native";
import { ask, common, create, engagement } from "@tokuly/api";
import type { ReactNode } from "react";
import { Alert, type AlertButton } from "react-native";
import { SafeAreaProvider } from "react-native-safe-area-context";
import { api } from "#/lib/api";
import { screenFor } from "#/seams/links";
import { AnswerScreen } from "../answer-screen";
import { AskForm } from "../ask-form";
import { InboxScreen } from "../inbox-screen";
import links from "../links";
import { ReportSheet } from "../parts";

jest.mock("#/lib/api", () => ({ api: jest.fn(), apiWith: jest.fn(), authClient: jest.fn() }));
/** Whether the screen is the one in front (another can sit on top of it in the stack). */
let mockFocused = true;
jest.mock("expo-router", () => ({
  router: { push: jest.fn(), back: jest.fn() },
  useIsFocused: () => mockFocused,
}));
/** Whether the account that started a piece of work is still the one in use. */
let mockActive = true;
jest.mock("#/features/session/account", () => {
  const { api } = jest.requireMock("#/lib/api");
  return {
    currentAccount: () => ({ sessionId: "s1", userId: "me", api: api() }),
    isActive: () => mockActive,
  };
});
beforeEach(() => {
  mockActive = true;
  mockFocused = true;
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
      getQuestion: async ({ questionId }: { questionId: string }) => ({
        question: questions.find((question) => question.id === questionId),
      }),
      getInboxCounts: async () => ({ counts: { unread: 1, inbox: 1 } }),
      markQuestionsRead: async (request: unknown) => {
        note("markQuestionsRead", request);
        return { counts: { unread: 0, inbox: 1 } };
      },
      blockSender: record("blockSender"),
      askQuestion: record("askQuestion"),
      createReport: record("createReport"),
    }),
  } as never);
  return sent;
}

const queryClient = new QueryClient();
afterEach(() => queryClient.clear());

const framed = (children: ReactNode) => (
  <SafeAreaProvider
    initialMetrics={{
      frame: { x: 0, y: 0, width: 320, height: 640 },
      insets: { top: 0, left: 0, right: 0, bottom: 0 },
    }}
  >
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  </SafeAreaProvider>
);

async function show(children: ReactNode) {
  await render(framed(children));
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

  it("reads only what is seen: not a question that arrives under another screen", async () => {
    const sent = server([anonymous]);
    mockFocused = false;
    await show(<InboxScreen />);
    expect(await screen.findByText("好きな食べ物は?")).toBeTruthy();
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(sent.markQuestionsRead).toBeUndefined();

    // Back on the inbox, it is on screen.
    mockFocused = true;
    await screen.rerender(framed(<InboxScreen />));
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

  it("never signs for an asker who began anonymously, when the box stops hiding them", async () => {
    const sent = server([]);
    await show(<AskForm box={box({ anonymousAvailable: true, allowSigned: true })} />);
    await fireEvent.changeText(screen.getByTestId("ask-input"), "すきな人いる?");
    // The owner lost a friend and the box was read again while the asker was writing.
    await screen.rerender(
      framed(<AskForm box={box({ anonymousAvailable: false, allowSigned: true })} />),
    );
    expect(screen.getByText(/名前つきでだけ送れるよ/)).toBeTruthy();
    await fireEvent.press(screen.getByTestId("ask-send"));
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(sent.askQuestion).toBeUndefined();

    await fireEvent.press(screen.getByTestId("ask-signed"));
    await fireEvent.press(screen.getByTestId("ask-send"));
    await waitFor(() => expect(sent.askQuestion?.[0]).toMatchObject({ signed: true }));
  });
});

describe("answering", () => {
  const question = (owner: string) =>
    create(ask.QuestionSchema, {
      id: "q1",
      text: "好きな食べ物は?",
      anonymous: true,
      owner: { id: owner, handle: owner, displayName: owner },
      folder: ask.QuestionFolder.INBOX,
    });

  it("is for the owner of the box", async () => {
    server([question("me")]);
    await show(<AnswerScreen questionId="q1" />);
    expect(await screen.findByTestId("ask-answer-screen")).toBeTruthy();
  });

  it("shows anyone else that there is nothing to see", async () => {
    server([question("u1")]);
    await show(<AnswerScreen questionId="q1" />);
    expect(await screen.findByText("この質問は見られません")).toBeTruthy();
    expect(screen.queryByTestId("ask-answer-screen")).toBeNull();
  });
});

describe("reporting", () => {
  it("sends the question's ID and the reason, nothing else", async () => {
    const sent = server([]);
    const alert = jest.spyOn(Alert, "alert").mockImplementation(() => {});
    await show(<ReportSheet questionId="q1" onClose={() => {}} />);
    await fireEvent.press(screen.getByTestId("sheet-report-HARASSMENT"));
    await waitFor(() => expect(alert).toHaveBeenCalledWith("通報したよ。教えてくれてありがとう！"));
    expect(sent.createReport).toEqual([
      {
        subject: { type: common.SubjectType.QUESTION, id: "q1" },
        category: engagement.ReportCategory.HARASSMENT,
        text: "",
      },
    ]);
    alert.mockRestore();
  });

  it("says nothing once another account took over", async () => {
    const sent = server([]);
    const alert = jest.spyOn(Alert, "alert").mockImplementation(() => {});
    await show(<ReportSheet questionId="q1" onClose={() => {}} />);
    mockActive = false;
    await fireEvent.press(screen.getByTestId("sheet-report-SPAM"));
    await waitFor(() => expect(sent.createReport).toHaveLength(1));
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(alert).not.toHaveBeenCalled();
    alert.mockRestore();
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
