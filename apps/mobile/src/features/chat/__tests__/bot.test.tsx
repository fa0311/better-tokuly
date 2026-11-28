import { QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor } from "@testing-library/react-native";
import { chat, create } from "@tokuly/api";
import { router } from "expo-router";
import { api } from "#/lib/api";
import { testQueryClient } from "#/testing/query";
import { BotMark, TalkToBotCard } from "../bot";

jest.mock("#/lib/api", () => ({ api: jest.fn(), apiWith: jest.fn(), authClient: jest.fn() }));
jest.mock("expo-router", () => ({ router: { replace: jest.fn() } }));

const BOT = { id: "bot", handle: "tokuly_bot", displayName: "Tokuly Bot", bot: true };

/** A server with the bot, whose switch for the viewer is `enabled`; records what was asked. */
function server(options: { bot: boolean; enabled: boolean }) {
  const asked: string[] = [];
  let enabled = options.enabled;
  const settings = () =>
    create(chat.AssistSettingsSchema, { bot: options.bot ? BOT : undefined, botEnabled: enabled });
  jest.mocked(api).mockReturnValue({
    service: () => ({
      getAssistSettings: async () => {
        asked.push("getAssistSettings");
        return { settings: settings() };
      },
      setBotEnabled: async (request: { enabled: boolean }) => {
        asked.push(`setBotEnabled:${request.enabled}`);
        enabled = request.enabled;
        return { settings: settings() };
      },
      openBotConversation: async () => {
        asked.push("openBotConversation");
        return { conversation: create(chat.ConversationSchema, { id: "c-bot" }) };
      },
    }),
  } as never);
  return asked;
}

async function show() {
  await render(
    <QueryClientProvider client={testQueryClient()}>
      <TalkToBotCard />
    </QueryClientProvider>,
  );
}

describe("TalkToBotCard", () => {
  it("opens the conversation with the bot", async () => {
    const asked = server({ bot: true, enabled: true });
    await show();
    fireEvent.press(await screen.findByTestId("talk-to-bot"));
    await waitFor(() =>
      expect(router.replace).toHaveBeenCalledWith({
        pathname: "/chat/[id]",
        params: { id: "c-bot" },
      }),
    );
    expect(asked).toEqual(["getAssistSettings", "openBotConversation"]);
    expect(screen.queryByTestId("bot-enabled")).toBeNull();
  });

  it("offers the switch while the bot is off, and opens nothing until then", async () => {
    const asked = server({ bot: true, enabled: false });
    await show();
    fireEvent.press(await screen.findByTestId("talk-to-bot"));
    fireEvent(screen.getByTestId("bot-enabled"), "valueChange", true);
    await waitFor(() => expect(screen.queryByTestId("bot-enabled")).toBeNull());
    expect(asked).toEqual(["getAssistSettings", "setBotEnabled:true"]);
  });

  it("shows nothing where the bot does not work", async () => {
    const asked = server({ bot: false, enabled: true });
    await show();
    await waitFor(() => expect(asked).toEqual(["getAssistSettings"]));
    expect(screen.queryByTestId("talk-to-bot")).toBeNull();
  });
});

describe("BotMark", () => {
  it("marks bots only", async () => {
    await render(<BotMark user={create(chat.MemberSchema).user} />);
    expect(screen.queryByTestId("bot-badge")).toBeNull();
    await render(<BotMark user={{ ...BOT, $typeName: "tokuly.common.v1.UserSummary" } as never} />);
    expect(screen.getByTestId("bot-badge")).toBeOnTheScreen();
  });
});
