import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor } from "@testing-library/react-native";
import { community, create } from "@tokuly/api";
import { SafeAreaProvider } from "react-native-safe-area-context";
import { api } from "#/lib/api";
import { ThreadScreen } from "../thread-screen";

jest.mock("#/lib/api", () => ({ api: jest.fn(), apiWith: jest.fn(), authClient: jest.fn() }));
jest.mock("expo-router", () => ({ router: { push: jest.fn() }, Stack: { Screen: () => null } }));

/** The boards as the server answers them, with the replies written. */
function server(member: boolean) {
  const written: { threadId: string; body: string; replyToId: string }[] = [];
  const replies = [
    create(community.ReplySchema, { id: "r1", threadId: "t1", number: 1, body: "よろしく！" }),
  ];
  jest.mocked(api).mockReturnValue({
    service: () => ({
      getCommunity: async () => ({
        community: create(community.CommunitySchema, {
          id: "c1",
          handle: "neko",
          name: "ねこ部",
          viewer: {
            status: member ? community.MembershipStatus.MEMBER : community.MembershipStatus.NONE,
          },
        }),
      }),
      getThread: async () => ({
        thread: create(community.ThreadSchema, {
          id: "t1",
          shortId: "abc",
          communityId: "c1",
          title: "はじめまして！",
          body: "ねこ好き集まれ",
          replyCount: replies.length,
          viewer: { canReply: member },
        }),
      }),
      listReplies: async () => ({ replies: [...replies], page: { nextCursor: "" } }),
      createReply: async (request: { threadId: string; body: string; replyToId: string }) => {
        written.push(request);
        const reply = create(community.ReplySchema, {
          id: "r2",
          threadId: "t1",
          number: 2,
          body: request.body,
        });
        replies.push(reply);
        return { reply };
      },
      getEngagement: async () => ({}),
    }),
  } as never);
  return { written };
}

async function show() {
  await render(
    <SafeAreaProvider
      initialMetrics={{
        frame: { x: 0, y: 0, width: 320, height: 640 },
        insets: { top: 0, left: 0, right: 0, bottom: 0 },
      }}
    >
      <QueryClientProvider client={new QueryClient()}>
        <ThreadScreen handle="neko" threadId="abc" />
      </QueryClientProvider>
    </SafeAreaProvider>,
  );
}

describe("ThreadScreen", () => {
  it("shows the thread and its replies, and answers one of them", async () => {
    const { written } = server(true);
    await show();
    expect(await screen.findByText("ねこ好き集まれ")).toBeOnTheScreen();
    expect(await screen.findByText("よろしく！")).toBeOnTheScreen();
    await fireEvent.press(screen.getByLabelText("#1に返信する"));
    await fireEvent.changeText(screen.getByTestId("reply-input"), "こちらこそ！");
    await fireEvent.press(screen.getByTestId("reply-send"));
    await waitFor(() =>
      expect(written).toEqual([{ threadId: "t1", body: "こちらこそ！", replyToId: "r1" }]),
    );
    expect(await screen.findByText("こちらこそ！")).toBeOnTheScreen();
  });

  it("asks someone outside to join before they reply", async () => {
    server(false);
    await show();
    expect(await screen.findByTestId("reply-blocked")).toHaveTextContent(
      "参加すると返信できるよ！",
    );
    expect(screen.queryByTestId("reply-input")).not.toBeOnTheScreen();
  });
});
