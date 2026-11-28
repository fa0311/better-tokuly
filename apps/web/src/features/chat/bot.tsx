import { useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import type { chat, common } from "@tokuly/api";
import { botHint } from "@tokuly/core/chat";
import { Bot, ChevronRight, MessageCircle } from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { SwitchRow } from "#/components/ui/switch";
import { cn } from "#/lib/cn";
import { useTranslation } from "#/lib/use-translation";
import { useAssistSettings, useAssistSettingsChange } from "./ai-settings";
import { assistClient, chatKeys, conversationClient } from "./api";
import { putConversation } from "./conversations";
import { errorMessage } from "./errors";

/**
 * Tokuly Bot, the AI account of the server: it answers in its direct
 * conversation, and in a group when mentioned (`@tokuly_bot`). The server
 * names it in the assistant settings (`bot`, unset where it does not work);
 * every bot account carries the bot badge (`UserSummary.bot`).
 */

/** The badge next to the name of a bot account. */
export function BotBadge({ className }: { className?: string }) {
  const { t } = useTranslation("chat");
  return (
    <span
      className={cn(
        "inline-flex h-5 shrink-0 items-center gap-0.5 rounded-full border-2 border-ink bg-sky px-1.5 text-[0.65rem] leading-none font-black text-[#17122b]",
        className,
      )}
      data-testid="bot-badge"
    >
      <Bot className="size-3" aria-hidden />
      {t("bot.badge")}
    </span>
  );
}

/** The badge when `user` is a bot, else nothing. */
export function BotMark({ user, className }: { user?: common.UserSummary; className?: string }) {
  return user?.bot && !user.deleted ? <BotBadge className={className} /> : null;
}

/** Opens the caller's conversation with the bot; resolves to it, or to `undefined` after a toast. */
export function useOpenBot(): () => Promise<chat.Conversation | undefined> {
  const queryClient = useQueryClient();
  return async () => {
    try {
      const { conversation } = await assistClient().openBotConversation({});
      if (!conversation) return undefined;
      // Enough to draw the screen at once; marked old so that opening it still asks for the extras.
      queryClient.setQueryData<chat.Conversation>(
        chatKeys.conversation(conversation.id),
        conversation,
        { updatedAt: 0 },
      );
      return conversation;
    } catch (error) {
      toast.error(errorMessage(error));
      return undefined;
    }
  };
}

/** "Talk to Tokuly Bot" for the new-chat dialog; nothing where the bot does not work. */
export function TalkToBotRow({ onOpen }: { onOpen(conversationId: string): void }) {
  const { t } = useTranslation("chat");
  const settings = useAssistSettings();
  const openBot = useOpenBot();
  const [opening, setOpening] = useState(false);
  const bot = settings.data?.bot;
  if (!bot || !settings.data?.botEnabled) return null;
  return (
    <button
      type="button"
      disabled={opening}
      onClick={async () => {
        setOpening(true);
        const conversation = await openBot();
        setOpening(false);
        if (conversation) onOpen(conversation.id);
      }}
      className="pressable sticker flex items-center gap-3 rounded-2xl bg-surface px-3 py-2.5 text-left disabled:opacity-60"
      data-testid="talk-to-bot"
    >
      <Avatar user={bot} outlined />
      <span className="min-w-0 flex-1">
        <span className="flex items-center gap-1.5 font-black">
          {t("bot.talk")}
          <BotBadge />
        </span>
        <span className="block truncate text-sm text-ink-soft">{t("bot.talkLead")}</span>
      </span>
      <ChevronRight className="size-5 shrink-0 text-ink-faint" />
    </button>
  );
}

/** The bot's part of the assistant settings: what it sends, the caller's switch, the way in. */
export function BotSettingsSection({
  settings,
  onOpen,
}: {
  settings: chat.AssistSettings;
  onOpen(conversationId: string): void;
}) {
  const { t } = useTranslation("chat");
  const openBot = useOpenBot();
  const [opening, setOpening] = useState(false);
  const change = useAssistSettingsChange((enabled: boolean) =>
    assistClient().setBotEnabled({ enabled }),
  );
  if (!settings.bot) return null;
  return (
    <section className="rounded-card bg-surface p-4" data-testid="bot-settings">
      <h2 className="flex items-center gap-2 font-black">
        <Bot className="size-4" />
        {t("bot.title")}
      </h2>
      <p className="mt-1 text-sm text-ink-soft">{t("bot.lead")}</p>
      <div className="-mx-2 mt-2">
        <SwitchRow
          label={t("bot.enabled")}
          hint={
            settings.botEnabledIsDefault
              ? `${t("bot.enabledHint")} ${t("bot.defaultNote")}`
              : t("bot.enabledHint")
          }
          checked={settings.botEnabled}
          disabled={change.isPending}
          onChange={(enabled) => change.mutate(enabled)}
        />
      </div>
      <Button
        className="mt-2"
        size="sm"
        icon={<MessageCircle className="size-4" />}
        loading={opening}
        disabled={!settings.botEnabled}
        onClick={async () => {
          setOpening(true);
          const conversation = await openBot();
          setOpening(false);
          if (conversation) onOpen(conversation.id);
        }}
      >
        {t("bot.talk")}
      </Button>
    </section>
  );
}

/** Above the composer of the bot's conversation while the caller's switch is off. */
export function BotOffBanner({ peer }: { peer?: common.UserSummary }) {
  const { t } = useTranslation("chat");
  const settings = useAssistSettings();
  const change = useAssistSettingsChange(
    () => assistClient().setBotEnabled({ enabled: true }),
    t("bot.turnedOn"),
  );
  const data = settings.data;
  if (!data?.bot || !peer || peer.id !== data.bot.id || data.botEnabled) return null;
  return (
    <div
      className="sticker mx-3 mb-2 flex items-center gap-3 rounded-2xl bg-sky px-4 py-3 text-[#17122b]"
      data-testid="bot-off"
    >
      <span className="min-w-0 flex-1">
        <span className="block font-black">{t("bot.offTitle")}</span>
        <span className="block text-sm">{t("bot.offBody")}</span>
      </span>
      <Button size="sm" loading={change.isPending} onClick={() => change.mutate(undefined)}>
        {t("bot.turnOn")}
      </Button>
    </div>
  );
}

/** Why the bot will not answer the writer's mention (`botHint`), shown only to the writer, never part of the conversation. */
export function BotMentionHint({ text }: { text: string }) {
  const { t } = useTranslation("chat");
  const settings = useAssistSettings();
  const change = useAssistSettingsChange(
    () => assistClient().setBotEnabled({ enabled: true }),
    t("bot.turnedOn"),
  );
  const hint = botHint(settings.data, text);
  if (!hint) return null;
  return (
    <p
      className="flex items-center gap-2 border-b-2 border-line bg-sky/40 px-4 py-2 text-sm font-bold"
      data-testid="bot-mention-hint"
    >
      <Bot className="size-4 shrink-0" aria-hidden />
      <span className="min-w-0 flex-1">
        {hint === "botOff" ? t("bot.mentionOff") : t("bot.mentionAssistOff")}
      </span>
      {hint === "botOff" ? (
        <Button
          size="sm"
          variant="ghost"
          loading={change.isPending}
          onClick={() => change.mutate(undefined)}
        >
          {t("bot.turnOn")}
        </Button>
      ) : (
        <Link
          to="/settings/chat-ai"
          className="shrink-0 text-brand underline-offset-2 hover:underline"
        >
          {t("bot.openAiSettings")}
        </Link>
      )}
    </p>
  );
}

/** "Invite Tokuly Bot" in a group's member list, while it is not a member. */
export function InviteBotButton({
  conversationId,
  memberIds,
  onAdded,
}: {
  conversationId: string;
  memberIds: string[];
  onAdded(): void;
}) {
  const { t } = useTranslation("chat");
  const queryClient = useQueryClient();
  const settings = useAssistSettings();
  const [adding, setAdding] = useState(false);
  const bot = settings.data?.bot;
  if (!bot || !settings.data?.botEnabled || memberIds.includes(bot.id)) return null;
  return (
    <Button
      size="sm"
      variant="ghost"
      icon={<Bot className="size-4" />}
      loading={adding}
      onClick={async () => {
        setAdding(true);
        try {
          const response = await conversationClient().addMembers({
            conversationId,
            userIds: [bot.id],
          });
          if (response.conversation) putConversation(queryClient, response.conversation);
          toast.success(t("bot.invited"));
          onAdded();
        } catch (error) {
          toast.error(errorMessage(error));
        } finally {
          setAdding(false);
        }
      }}
    >
      {t("bot.invite")}
    </Button>
  );
}
