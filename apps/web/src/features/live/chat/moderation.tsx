import { useMutation, useQueryClient } from "@tanstack/react-query";
import { common, live } from "@tokuly/api";
import { AtSign, Ban, Flag, Pin, Timer, Trash2 } from "lucide-react";
import { type ReactNode, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Dialog } from "#/components/ui/dialog";
import { TextField } from "#/components/ui/field";
import { SwitchRow } from "#/components/ui/switch";
import { ReportDialog } from "#/features/engagement";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { chatClient, liveKeys } from "../api";
import { authorName, mentionOf } from "./chat-line";

const { ChatMessageKind, ChatRole } = live;

/** How long a timeout lasts, in seconds; 0 is a ban for good. `key` is `moderation.duration.<key>`. */
const DURATIONS = [
  { seconds: 60, key: "minute" },
  { seconds: 600, key: "tenMinutes" },
  { seconds: 3600, key: "hour" },
  { seconds: 86_400, key: "day" },
  { seconds: 0, key: "ban" },
] as const;

/**
 * Times a person out of a channel's chats, or bans them for good. For the
 * streamer and the channel's moderators.
 */
export function RestrictDialog({
  channelId,
  user,
  open,
  onOpenChange,
}: {
  channelId: string;
  user: common.UserSummary | undefined;
  open: boolean;
  onOpenChange(open: boolean): void;
}) {
  const { t } = useTranslation("live");
  const queryClient = useQueryClient();
  const [seconds, setSeconds] = useState<number>(600);
  const [reason, setReason] = useState("");
  const [deleteMessages, setDeleteMessages] = useState(false);
  const restrict = useMutation({
    mutationFn: () =>
      chatClient().restrictChatUser({
        channelId,
        userId: user?.id ?? "",
        durationSeconds: seconds,
        reason: reason.trim(),
        deleteMessages,
      }),
    onSuccess: () => {
      toast.success(
        t(seconds === 0 ? "moderation.banned" : "moderation.timedOut", {
          name: user?.displayName ?? "",
        }),
      );
      void queryClient.invalidateQueries({ queryKey: liveKeys.restrictions(channelId) });
      onOpenChange(false);
      setReason("");
      setDeleteMessages(false);
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "live" })),
  });
  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("moderation.restrictTitle", { name: user?.displayName ?? "" })}
      description={t("moderation.restrictLead")}
      footer={
        <>
          <Button variant="ghost" onClick={() => onOpenChange(false)}>
            {t("common:actions.cancel")}
          </Button>
          <Button
            variant="danger"
            loading={restrict.isPending}
            onClick={() => restrict.mutate()}
            icon={seconds === 0 ? <Ban className="size-4" /> : <Timer className="size-4" />}
          >
            {t(seconds === 0 ? "moderation.ban" : "moderation.timeout")}
          </Button>
        </>
      }
    >
      <fieldset className="flex flex-wrap gap-2">
        <legend className="mb-2 px-1 text-sm font-bold text-ink-soft">
          {t("moderation.durationLabel")}
        </legend>
        {DURATIONS.map((option) => (
          <button
            key={option.key}
            type="button"
            aria-pressed={seconds === option.seconds}
            onClick={() => setSeconds(option.seconds)}
            className={cn(
              "h-10 rounded-full border-2 px-4 text-sm font-black",
              seconds === option.seconds
                ? "border-ink bg-sun text-[#17122b]"
                : "border-line bg-surface text-ink-soft hover:text-ink",
            )}
          >
            {t(`moderation.duration.${option.key}`)}
          </button>
        ))}
      </fieldset>
      <TextField
        className="mt-4"
        label={t("moderation.reason")}
        hint={t("moderation.reasonHint")}
        value={reason}
        maxLength={200}
        onChange={(event) => setReason(event.target.value)}
      />
      <div className="mt-2">
        <SwitchRow
          label={t("moderation.deleteMessages")}
          hint={t("moderation.deleteMessagesHint")}
          checked={deleteMessages}
          onChange={setDeleteMessages}
        />
      </div>
    </Dialog>
  );
}

/**
 * What can be done with one chat line: mention its author, report it, and —
 * for those who may — delete it, pin it, time its author out or ban them.
 */
export function MessageActions({
  message,
  stream,
  selfId,
  onClose,
  onMention,
}: {
  /** The line the sheet is open for; `null` closes it. */
  message: live.ChatMessage | null;
  stream: live.Stream;
  selfId: string | undefined;
  onClose(): void;
  onMention(message: live.ChatMessage): void;
}) {
  const { t } = useTranslation("live");
  const [reporting, setReporting] = useState<live.ChatMessage | null>(null);
  const [restricting, setRestricting] = useState<common.UserSummary | null>(null);
  const viewer = stream.viewer;
  const channelId = stream.summary?.channel?.id ?? "";
  const author = message?.author?.user;
  const own = author !== undefined && author.id === selfId;
  const byStreamer = message?.role === ChatRole.OWNER;
  const isGift = message?.kind === ChatMessageKind.GIFT;
  const moderator = Boolean(viewer?.isModerator);
  const owner = Boolean(viewer?.isOwner);
  // The streamer's lines are the streamer's alone to delete; gifts need a moderator.
  const mayDelete = byStreamer ? owner : moderator || (own && !isGift);
  const mayRestrict =
    moderator &&
    author !== undefined &&
    !own &&
    !byStreamer &&
    (owner || message?.role !== ChatRole.MODERATOR);

  const remove = useMutation({
    mutationFn: (messageId: string) => chatClient().deleteChatMessage({ messageId }),
    onSuccess: () => {
      toast.success(t("chat.deleted"));
      onClose();
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "live" })),
  });
  const pin = useMutation({
    mutationFn: (messageId: string) => chatClient().pinChatMessage({ messageId }),
    onSuccess: () => {
      toast.success(t("chat.pinned"));
      onClose();
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "live" })),
  });

  return (
    <>
      <Dialog
        open={message !== null}
        onOpenChange={(open) => !open && onClose()}
        title={t("chat.actionsTitle", { name: message ? authorName(message) : "" })}
        description={message?.text ? message.text.slice(0, 80) : undefined}
      >
        {message && (
          <div className="flex flex-col gap-1">
            {mentionOf(message) && !own && (
              <ActionRow
                icon={<AtSign />}
                label={t("chat.mention")}
                onClick={() => {
                  onMention(message);
                  onClose();
                }}
              />
            )}
            {moderator && message.kind !== ChatMessageKind.NOTICE && (
              <ActionRow
                icon={<Pin />}
                label={t("chat.pin")}
                disabled={pin.isPending}
                onClick={() => pin.mutate(message.id)}
              />
            )}
            {mayDelete && (
              <ActionRow
                icon={<Trash2 />}
                label={t("chat.delete")}
                danger
                disabled={remove.isPending}
                onClick={() => remove.mutate(message.id)}
              />
            )}
            {mayRestrict && author && (
              <ActionRow
                icon={<Timer />}
                label={t("moderation.restrictAction")}
                danger
                onClick={() => {
                  setRestricting(author);
                  onClose();
                }}
              />
            )}
            {!own && !owner && (
              <ActionRow
                icon={<Flag />}
                label={t("chat.report")}
                onClick={() => {
                  setReporting(message);
                  onClose();
                }}
              />
            )}
          </div>
        )}
      </Dialog>
      <RestrictDialog
        channelId={channelId}
        user={restricting ?? undefined}
        open={restricting !== null}
        onOpenChange={(open) => !open && setRestricting(null)}
      />
      {reporting && (
        <ReportDialog
          subject={{ type: common.SubjectType.LIVE_CHAT_MESSAGE, id: reporting.id }}
          open
          onOpenChange={(open) => !open && setReporting(null)}
          label={`${authorName(reporting)}: ${reporting.text.slice(0, 40)}`}
        />
      )}
    </>
  );
}

function ActionRow({
  icon,
  label,
  danger,
  disabled,
  onClick,
}: {
  icon: ReactNode;
  label: string;
  danger?: boolean;
  disabled?: boolean;
  onClick(): void;
}) {
  return (
    <button
      type="button"
      disabled={disabled}
      onClick={onClick}
      className={cn(
        "flex h-12 w-full items-center gap-3 rounded-2xl px-3 text-left font-bold hover:bg-surface-2 disabled:opacity-50 [&>svg]:size-5",
        danger && "text-danger",
      )}
    >
      {icon}
      {label}
    </button>
  );
}
