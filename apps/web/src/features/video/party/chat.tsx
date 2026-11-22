import { common, toMillis, video } from "@tokuly/api";
import { Flag, SendHorizontal, Smile } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { EmojiText } from "#/features/emoji";
import { ReportDialog } from "#/features/engagement";
import { StampImage, StampPicker } from "#/features/stamps";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import type { PartyNotice, PartyRoom } from "./use-party";

const { PartyRole } = video;
const MAX_LENGTH = 500;

/** A member's face: their avatar, or — for a guest — their initial on a tint of their own. */
export function MemberAvatar({
  member,
  size = "sm",
}: {
  member: video.PartyMember;
  size?: "xs" | "sm" | "md";
}) {
  return (
    <Avatar
      user={
        member.user ?? {
          id: member.id,
          displayName: member.name,
          handle: member.name,
          avatar: undefined,
          deleted: false,
        }
      }
      size={size}
    />
  );
}

type Entry =
  | { kind: "message"; atMs: number; message: video.PartyMessage }
  | { kind: "notice"; atMs: number; notice: PartyNotice };

/**
 * The party's chat: messages and stamps as they arrive, with a quiet line
 * when someone comes or goes. It keeps to the newest message unless the
 * reader has scrolled up to look at older ones.
 */
export function PartyChat({
  room,
  canStamp,
}: {
  room: PartyRoom /** Stamps need an account; guests write text only. */;
  canStamp: boolean;
}) {
  const { t, i18n } = useTranslation("video");
  const { messages, notices, self } = room.state;
  const list = useRef<HTMLDivElement>(null);
  const pinned = useRef(true);
  const [text, setText] = useState("");
  const [sending, setSending] = useState(false);
  const [reporting, setReporting] = useState<video.PartyMessage | null>(null);

  const entries: Entry[] = [
    ...messages.map(
      (message): Entry => ({ kind: "message", atMs: toMillis(message.sentAt), message }),
    ),
    ...notices.map((notice): Entry => ({ kind: "notice", atMs: notice.atMs, notice })),
  ].sort((a, b) => a.atMs - b.atMs);

  const count = entries.length;
  // follow the newest entry whenever one is added.
  useEffect(() => {
    const box = list.current;
    if (box && pinned.current) box.scrollTop = box.scrollHeight;
  }, [count]);

  const send = async (message: { text?: string; stampId?: string }) => {
    setSending(true);
    try {
      await room.sendMessage(message);
      if (message.text) setText("");
      pinned.current = true;
    } catch (error) {
      toast.error(errorMessage(error));
    } finally {
      setSending(false);
    }
  };
  const time = (atMs: number) =>
    new Date(atMs).toLocaleTimeString(i18n.language, { hour: "2-digit", minute: "2-digit" });

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div
        ref={list}
        role="log"
        aria-label={t("party.chat.title")}
        data-testid="party-chat"
        className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto px-4 py-3"
        onScroll={(event) => {
          const box = event.currentTarget;
          pinned.current = box.scrollHeight - box.scrollTop - box.clientHeight < 48;
        }}
      >
        {entries.length === 0 && (
          <p className="m-auto max-w-56 text-center text-sm leading-relaxed text-ink-faint">
            {t("party.chat.empty")}
          </p>
        )}
        {entries.map((entry) => {
          if (entry.kind === "notice") {
            return (
              <p key={entry.notice.id} className="text-center text-xs font-bold text-ink-faint">
                {t(`party.chat.${entry.notice.kind}`, { name: entry.notice.name })}
              </p>
            );
          }
          const { message } = entry;
          const sender = message.sender;
          const mine = sender?.id === self.id;
          return (
            <div
              key={message.id}
              data-testid="party-message"
              className={cn("flex items-start gap-2.5", mine && "flex-row-reverse")}
            >
              {sender && <MemberAvatar member={sender} />}
              <div
                className={cn(
                  "flex min-w-0 max-w-[80%] flex-col",
                  mine ? "items-end" : "items-start",
                )}
              >
                <p className="flex max-w-full items-center gap-1.5 text-xs text-ink-soft">
                  <span className="truncate font-bold">{sender?.name}</span>
                  {sender?.role === PartyRole.HOST && (
                    <span className="shrink-0 rounded-full bg-sun px-1.5 text-[10px] leading-4 font-black text-[#17122b]">
                      {t("party.role.host")}
                    </span>
                  )}
                  <span className="shrink-0 text-ink-faint">{time(entry.atMs)}</span>
                  {/* Party messages are not kept as reportable subjects: the report is about who wrote it, quoting it. */}
                  {!mine && sender?.user && (
                    <button
                      type="button"
                      aria-label={t("party.chat.report", { name: sender.name })}
                      title={t("party.chat.report", { name: sender.name })}
                      className="grid size-6 shrink-0 place-items-center rounded-full text-ink-faint hover:bg-surface-2 hover:text-ink"
                      onClick={() => setReporting(message)}
                    >
                      <Flag className="size-3.5" />
                    </button>
                  )}
                </p>
                {message.text && (
                  <p
                    className={cn(
                      "mt-0.5 rounded-2xl px-3 py-1.5 text-[15px] leading-relaxed break-words whitespace-pre-wrap",
                      mine ? "bg-brand text-on-brand" : "bg-surface-2",
                    )}
                  >
                    <EmojiText text={message.text} />
                  </p>
                )}
                {message.stamp && <StampImage stamp={message.stamp} size={96} className="mt-1" />}
              </div>
            </div>
          );
        })}
      </div>
      <form
        className="flex items-center gap-2 border-t-2 border-line px-3 py-2"
        onSubmit={(event) => {
          event.preventDefault();
          if (text.trim() && !sending) void send({ text: text.trim() });
        }}
      >
        {canStamp && (
          <StampPicker
            trigger={
              <Button variant="ghost" size="icon-sm" aria-label={t("party.chat.stamp")}>
                <Smile className="size-5" />
              </Button>
            }
            onPick={(stamp) => void send({ stampId: stamp.id })}
          />
        )}
        <input
          value={text}
          maxLength={MAX_LENGTH}
          placeholder={t("party.chat.placeholder")}
          aria-label={t("party.chat.placeholder")}
          className="h-10 min-w-0 flex-1 rounded-full border-2 border-line bg-surface px-4 text-[15px] outline-none placeholder:text-ink-faint focus:border-brand"
          onChange={(event) => setText(event.target.value)}
        />
        <Button
          type="submit"
          size="icon-sm"
          aria-label={t("party.chat.send")}
          disabled={!text.trim() || sending}
        >
          <SendHorizontal className="size-4" />
        </Button>
      </form>
      {reporting?.sender?.user && (
        <ReportDialog
          subject={{ type: common.SubjectType.USER, id: reporting.sender.user.id }}
          open
          onOpenChange={(open) => !open && setReporting(null)}
          label={`${reporting.sender.name}: ${reporting.text || t("party.chat.stamp")}`}
        />
      )}
    </div>
  );
}
