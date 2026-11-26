import { engagement, live } from "@tokuly/api";
import type { Subject } from "@tokuly/core/engagement";
import { authorName, mentionOf } from "@tokuly/core/live";
import { useTranslation } from "react-i18next";
import { Alert } from "react-native";
import { ActionSheet, type SheetAction } from "#/components/action-sheet";
import { api } from "#/lib/api";
import { errorMessage } from "#/lib/errors";
import { chatClient } from "./data";

const { ChatMessageKind, ChatRole } = live;
const { ReportCategory } = engagement;

/**
 * Opens one sheet after another has gone: iOS does not present a modal while
 * the previous one is still sliding away.
 */
export function afterSheet(open: () => void): void {
  setTimeout(open, 400);
}

/** The categories in the order they are offered; their names are `live:app.report.categories.<key>`. */
const REPORT_CATEGORIES = [
  ["SPAM", ReportCategory.SPAM],
  ["HARASSMENT", ReportCategory.HARASSMENT],
  ["HATE", ReportCategory.HATE],
  ["SEXUAL", ReportCategory.SEXUAL],
  ["VIOLENCE", ReportCategory.VIOLENCE],
  ["SELF_HARM", ReportCategory.SELF_HARM],
  ["ILLEGAL", ReportCategory.ILLEGAL],
  ["IMPERSONATION", ReportCategory.IMPERSONATION],
  ["COPYRIGHT", ReportCategory.COPYRIGHT],
  ["OTHER", ReportCategory.OTHER],
] as const;

/**
 * Reports a stream or a chat line to the moderation team: pick what is
 * wrong, and it is sent (as the account that picked it; the client is taken
 * before anything is awaited).
 */
export function ReportSheet({
  subject,
  onClose,
}: {
  /** What is reported; `null` closes the sheet. */
  subject: Subject | null;
  onClose(): void;
}) {
  const { t } = useTranslation("live");
  const send = async (category: engagement.ReportCategory) => {
    if (!subject) return;
    const reports = api().service(engagement.ReportService);
    try {
      await reports.createReport({ subject, category, text: "" });
      Alert.alert(t("app.report.sent"));
    } catch (error) {
      Alert.alert(errorMessage(error, { namespace: "live" }));
    }
  };
  return (
    <ActionSheet
      visible={subject !== null}
      title={t("app.report.title")}
      onClose={onClose}
      actions={REPORT_CATEGORIES.map(([key, category]) => ({
        key,
        label: t(`app.report.categories.${key}`),
        onPress: () => void send(category),
      }))}
    />
  );
}

/**
 * What can be done with one chat line: reply to its author, report it, and
 * — for those who may, by the same rules as the server — pin or delete it.
 * Timeouts and bans are the web's chat moderation.
 */
export function MessageActions({
  message,
  stream,
  selfId,
  onClose,
  onMention,
  onReport,
}: {
  /** The line the sheet is open for; `null` closes it. */
  message: live.ChatMessage | null;
  stream: live.Stream;
  selfId: string | undefined;
  onClose(): void;
  onMention(message: live.ChatMessage): void;
  onReport(message: live.ChatMessage): void;
}) {
  const { t } = useTranslation("live");
  const viewer = stream.viewer;
  const moderator = Boolean(viewer?.isModerator);
  const owner = Boolean(viewer?.isOwner);
  const actions: SheetAction[] = [];
  if (message) {
    const author = message.author?.user;
    const own = author !== undefined && author.id === selfId;
    const byStreamer = message.role === ChatRole.OWNER;
    const isGift = message.kind === ChatMessageKind.GIFT;
    // The streamer's lines are the streamer's alone to delete; gifts need a moderator.
    const mayDelete = byStreamer ? owner : moderator || (own && !isGift);
    if (mentionOf(message) && !own)
      actions.push({ key: "mention", label: t("chat.mention"), onPress: () => onMention(message) });
    if (moderator && message.kind !== ChatMessageKind.NOTICE)
      actions.push({
        key: "pin",
        label: t("chat.pin"),
        onPress: () =>
          void moderate(
            () => chatClient().pinChatMessage({ messageId: message.id }),
            t("chat.pinned"),
          ),
      });
    if (mayDelete)
      actions.push({
        key: "delete",
        label: t("chat.delete"),
        destructive: true,
        onPress: () =>
          void moderate(
            () => chatClient().deleteChatMessage({ messageId: message.id }),
            t("chat.deleted"),
          ),
      });
    if (!own && !owner)
      actions.push({ key: "report", label: t("chat.report"), onPress: () => onReport(message) });
  }
  return (
    <ActionSheet
      visible={message !== null}
      title={message ? t("chat.actionsTitle", { name: authorName(message) }) : undefined}
      actions={actions}
      onClose={onClose}
    />
  );
}

/** Pins or deletes a line; the push brings the change to every screen, this one included. */
async function moderate(work: () => Promise<unknown>, done: string): Promise<void> {
  try {
    await work();
    Alert.alert(done);
  } catch (error) {
    Alert.alert(errorMessage(error, { namespace: "live" }));
  }
}
