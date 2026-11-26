import { useMutation, useQueryClient } from "@tanstack/react-query";
import { errorReason, type live, toMillis } from "@tokuly/api";
import { slowModeWait } from "@tokuly/core/live";
import { Gift, Hourglass, SendHorizontal, Sticker } from "lucide-react";
import { type Ref, useEffect, useImperativeHandle, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { EmojiSuggestBar, recordEmojiUse, shortcodesIn } from "#/features/emoji";
import { StampPicker } from "#/features/stamps";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { chatClient } from "../api";
import { useNow } from "../parts";
import { addOwnMessage } from "../room";

const MAX_LENGTH = 200;

export interface ComposerHandle {
  /** Puts text (a mention) at the start of the field and focuses it. */
  insert(text: string): void;
}

/**
 * Where a member writes into the live chat: text with emoji suggestions, a
 * stamp, a gift. It says why when writing is not possible (a timeout, a chat
 * for subscribers only) and counts slow mode down.
 */
export function ChatComposer({
  stream,
  onGift,
  ref,
}: {
  stream: live.Stream;
  /** Opens the gift dialog; left out where gifts cannot be sent. */
  onGift?(): void;
  ref?: Ref<ComposerHandle>;
}) {
  const { t, i18n } = useTranslation("live");
  const queryClient = useQueryClient();
  const input = useRef<HTMLInputElement>(null);
  const [text, setText] = useState("");
  const [caret, setCaret] = useState(0);
  /** When the last message went out, for slow mode. */
  const [sentAt, setSentAt] = useState<number | null>(null);
  const inFlight = useRef(false);
  const now = useNow();
  const streamId = stream.summary?.id ?? "";
  const viewer = stream.viewer;
  const exempt = Boolean(viewer?.isModerator);
  const wait = exempt ? 0 : slowModeWait(stream.chat?.slowModeSeconds ?? 0, sentAt, now);

  useImperativeHandle(ref, () => ({
    insert(mention) {
      setText((current) => (current.startsWith(mention) ? current : mention + current));
      requestAnimationFrame(() => input.current?.focus());
    },
  }));

  const moveCaret = (position: number) =>
    requestAnimationFrame(() => {
      input.current?.setSelectionRange(position, position);
      setCaret(position);
    });

  const send = useMutation({
    mutationFn: (payload: { text: string } | { stampId: string }) =>
      chatClient().sendChatMessage({ streamId, text: "", stampId: "", ...payload }),
    onSuccess: ({ message }, payload) => {
      setSentAt(Date.now());
      if ("text" in payload) {
        setText("");
        recordEmojiUse(shortcodesIn(payload.text));
      }
      if (message) addOwnMessage(queryClient, message);
    },
    onError: (error) => {
      // The server's clock decides: wait the whole interval again.
      if (errorReason(error) === "SLOW_MODE") setSentAt(Date.now());
      toast.error(errorMessage(error, { namespace: "live" }));
    },
  });

  // Why the viewer cannot write, as the server last said; a timeout that ran out no longer counts.
  const blockedUntil = viewer?.chatBlockedUntil ? toMillis(viewer.chatBlockedUntil) : undefined;
  const timeoutOver = viewer?.chatBlockReason === "CHAT_TIMED_OUT" && (blockedUntil ?? 0) <= now;
  const blocked = viewer && !viewer.canChat && !timeoutOver ? viewer.chatBlockReason : "";
  useEffect(() => {
    if (blocked) setText("");
  }, [blocked]);

  if (blocked) {
    return (
      <p
        className="mx-3 mb-3 rounded-2xl bg-surface-2 px-4 py-3 text-center text-sm font-bold text-ink-soft"
        data-testid="chat-blocked"
      >
        {blocked === "CHAT_TIMED_OUT" && blockedUntil
          ? t("chat.blocked.CHAT_TIMED_OUT", {
              time: new Date(blockedUntil).toLocaleTimeString(i18n.language, {
                hour: "2-digit",
                minute: "2-digit",
              }),
            })
          : t(`chat.blocked.${blocked}`, { defaultValue: t("chat.blocked.other") })}
      </p>
    );
  }

  const trimmed = text.trim();
  const canSend = trimmed.length > 0 && wait === 0 && !send.isPending;
  const submit = () => {
    // A second submit in the same frame (before `isPending` shows) sends nothing.
    if (!canSend || inFlight.current) return;
    inFlight.current = true;
    send.mutate(
      { text: trimmed },
      {
        onSettled: () => {
          inFlight.current = false;
        },
      },
    );
  };

  return (
    <div className="border-t-2 border-line px-3 pt-2 pb-3">
      <EmojiSuggestBar
        text={text}
        caret={caret}
        onComplete={(next) => {
          setText(next.text);
          moveCaret(next.caret);
        }}
        className="mb-2"
      />
      {wait > 0 && (
        <p
          className="mb-1.5 flex items-center gap-1.5 px-1 text-xs font-bold text-ink-soft"
          role="timer"
          data-testid="slow-mode-wait"
        >
          <Hourglass className="size-3.5" />
          {t("chat.slowWait", { count: wait })}
        </p>
      )}
      <form
        className="flex items-center gap-1.5"
        onSubmit={(event) => {
          event.preventDefault();
          submit();
        }}
      >
        <div className="flex h-11 min-w-0 flex-1 items-center rounded-full border-2 border-line bg-surface pr-1 pl-4 focus-within:border-brand">
          <input
            ref={input}
            value={text}
            maxLength={MAX_LENGTH}
            onChange={(event) => {
              setText(event.target.value);
              setCaret(event.target.selectionStart ?? event.target.value.length);
            }}
            onSelect={(event) => setCaret(event.currentTarget.selectionStart ?? 0)}
            placeholder={
              stream.chat?.slowModeSeconds && !exempt
                ? t("chat.placeholderSlow", { count: stream.chat.slowModeSeconds })
                : t("chat.placeholder")
            }
            aria-label={t("chat.inputLabel")}
            enterKeyHint="send"
            autoComplete="off"
            className="h-full min-w-0 flex-1 bg-transparent text-[15px] outline-none placeholder:text-ink-faint"
          />
          {stream.chat?.stampsEnabled && (
            <StampPicker
              onPick={(stamp) => send.mutate({ stampId: stamp.id })}
              trigger={
                <button
                  type="button"
                  aria-label={t("chat.stamp")}
                  disabled={wait > 0}
                  className="grid size-9 shrink-0 place-items-center rounded-full text-ink-soft hover:bg-surface-2 hover:text-ink disabled:opacity-40"
                >
                  <Sticker className="size-5" />
                </button>
              }
            />
          )}
        </div>
        {onGift && (
          <button
            type="button"
            onClick={onGift}
            aria-label={t("gift.open")}
            className="pressable sticker-sm grid size-11 shrink-0 place-items-center rounded-full bg-sun text-[#17122b]"
          >
            <Gift className="size-5" />
          </button>
        )}
        <button
          type="submit"
          aria-label={t("chat.send")}
          disabled={!canSend}
          className={cn(
            "pressable sticker-sm grid size-11 shrink-0 place-items-center rounded-full bg-brand text-on-brand disabled:opacity-40",
          )}
        >
          <SendHorizontal className="size-5" />
        </button>
      </form>
    </div>
  );
}
