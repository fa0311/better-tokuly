import { type QueryClient, useQueryClient } from "@tanstack/react-query";
import { chat, toMillis } from "@tokuly/api";
import { peerOf } from "@tokuly/core/chat";
import { BookmarkCheck, Camera, Clock } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Dialog } from "#/components/ui/dialog";
import { TextArea } from "#/components/ui/field";
import { povClient } from "./api";
import { useConversationContext } from "./conversation-context";
import { errorMessage } from "./errors";
import { useLive } from "./live";
import { readTimeline, storeCreated, storeUpdated } from "./messages";
import { displayName } from "./people";
import { playTime } from "./text";

/** Milliseconds until `targetMs`, counting down once a second (0 when it has passed or is unset). */
export function useCountdown(targetMs: number): number {
  const [now, setNow] = useState(() => Date.now());
  const left = Math.max(0, targetMs - now);
  const running = targetMs > 0 && left > 0;
  useEffect(() => {
    if (!running) return;
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [running]);
  return targetMs > 0 ? left : 0;
}

const NOTE_IDEAS = ["now", "doing", "show"] as const;

/** "Ask for a POV": an optional short note, then the request goes into the conversation. */
export function PovRequestDialog({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange(open: boolean): void;
}) {
  const { t } = useTranslation("chat");
  const queryClient = useQueryClient();
  const { conversation, me, capabilities } = useConversationContext();
  const [note, setNote] = useState("");
  const [sending, setSending] = useState(false);
  const peer = peerOf(conversation, me.id);

  const request = async () => {
    setSending(true);
    try {
      const response = await povClient().requestPov({
        conversationId: conversation.id,
        note: note.trim(),
      });
      if (response.message) storeCreated(queryClient, response.message);
      useLive.getState().setOpenPov(conversation.id, response.request);
      setNote("");
      onOpenChange(false);
    } catch (error) {
      toast.error(errorMessage(error));
    } finally {
      setSending(false);
    }
  };

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("pov.requestTitle")}
      description={t("pov.requestLead", {
        name: displayName(peer),
        hours: Math.round(capabilities.povAnswerSeconds / 3600),
      })}
      footer={
        <Button onClick={request} loading={sending} icon={<Camera className="size-4" />}>
          {t("pov.request")}
        </Button>
      }
    >
      <div className="flex flex-col gap-3">
        <div className="flex flex-wrap gap-2">
          {NOTE_IDEAS.map((idea) => (
            <button
              key={idea}
              type="button"
              onClick={() => setNote(t(`pov.ideas.${idea}`))}
              className="pressable sticker-sm rounded-full bg-surface px-3 py-1 text-sm font-bold"
            >
              {t(`pov.ideas.${idea}`)}
            </button>
          ))}
        </div>
        <TextArea
          label={t("pov.note")}
          placeholder={t("pov.notePlaceholder")}
          value={note}
          maxLength={200}
          onChange={(event) => setNote(event.target.value)}
        />
      </div>
    </Dialog>
  );
}

/** What happened to a POV request is shown at once, without waiting for the event that follows. */
function storeRequest(queryClient: QueryClient, request: chat.PovRequest): void {
  useLive.getState().setOpenPov(request.conversationId, request);
  const messages = readTimeline(queryClient, request.conversationId)?.messages ?? [];
  const asked = messages.find((message) => message.id === request.requestMessageId);
  if (asked) storeUpdated(queryClient, { ...asked, povRequest: request }, false);
  const answer = messages.find((message) => message.id === request.replyMessageId);
  if (answer?.povReply && request.saved)
    storeUpdated(
      queryClient,
      { ...answer, povReply: { ...answer.povReply, saved: true, expiresAt: undefined } },
      false,
    );
}

/** The actions on a POV request: decline (the person asked), cancel or save the answer (the requester). */
export function usePovActions(requestId: string | undefined) {
  const queryClient = useQueryClient();
  const [busy, setBusy] = useState(false);

  const run = async (work: (requestId: string) => Promise<{ request?: chat.PovRequest }>) => {
    if (!requestId) return;
    setBusy(true);
    try {
      const response = await work(requestId);
      if (response.request) storeRequest(queryClient, response.request);
    } catch (error) {
      toast.error(errorMessage(error));
    } finally {
      setBusy(false);
    }
  };

  return {
    busy,
    decline: () => run((id) => povClient().declinePov({ requestId: id })),
    cancel: () => run((id) => povClient().cancelPov({ requestId: id })),
    save: () => run((id) => povClient().savePov({ requestId: id })),
  };
}

/** A POV request in the conversation, with what the viewer can do about it. */
export function PovRequestCard({ message }: { message: chat.Message }) {
  const { t } = useTranslation("chat");
  const { conversation, me, writable, jumpTo, answerPov } = useConversationContext();
  const request = message.povRequest;
  const actions = usePovActions(request?.id);
  const left = useCountdown(
    request?.state === chat.PovState.OPEN ? toMillis(request.expiresAt) : 0,
  );
  if (!request) return null;
  const mine = request.requesterId === me.id;
  const peer = peerOf(conversation, me.id);
  const open = request.state === chat.PovState.OPEN && left > 0;

  const status = (() => {
    switch (request.state) {
      case chat.PovState.OPEN:
        return left > 0
          ? t("pov.timeLeft", { time: playTime(left / 1000) })
          : t("pov.state.expired");
      case chat.PovState.ANSWERED:
        return t("pov.state.answered");
      case chat.PovState.DECLINED:
        return mine
          ? t("pov.state.declinedByThem", { name: displayName(peer) })
          : t("pov.state.declined");
      case chat.PovState.CANCELLED:
        return t("pov.state.cancelled");
      default:
        return t("pov.state.expired");
    }
  })();

  return (
    <div
      className="mx-auto flex w-72 max-w-full flex-col items-center gap-2 rounded-card border-2 border-line bg-surface p-4 text-center"
      data-testid="pov-request"
    >
      <span className="sticker-sm grid size-12 place-items-center rounded-2xl bg-sun text-[#17122b]">
        <Camera className="size-6" />
      </span>
      <p className="font-black">
        {mine
          ? t("pov.askedByMe", { name: displayName(peer) })
          : t("pov.askedByThem", { name: displayName(peer) })}
      </p>
      {request.note && <p className="text-sm text-ink-soft">「{request.note}」</p>}
      <p
        className={`flex items-center gap-1 text-xs font-bold tabular-nums ${open ? "text-brand" : "text-ink-faint"}`}
      >
        {open && <Clock className="size-3.5" />}
        {status}
      </p>
      {open && writable && !mine && (
        <div className="mt-1 flex flex-wrap justify-center gap-2">
          <Button
            size="sm"
            variant="lime"
            icon={<Camera className="size-4" />}
            onClick={() => answerPov(request.id)}
          >
            {t("pov.answer")}
          </Button>
          <Button size="sm" variant="ghost" loading={actions.busy} onClick={actions.decline}>
            {t("pov.decline")}
          </Button>
        </div>
      )}
      {open && mine && (
        <Button size="sm" variant="ghost" loading={actions.busy} onClick={actions.cancel}>
          {t("pov.cancel")}
        </Button>
      )}
      {request.state === chat.PovState.ANSWERED && request.replyMessageId && (
        <Button size="sm" variant="secondary" onClick={() => jumpTo(request.replyMessageId)}>
          {t("pov.view")}
        </Button>
      )}
    </div>
  );
}

/** The line under a POV answer: how long it still lives, and "save" for the person who asked. */
export function PovReplyFooter({
  message,
  tone,
}: {
  message: chat.Message;
  tone: "own" | "other";
}) {
  const { t } = useTranslation("chat");
  const { me } = useConversationContext();
  const reply = message.povReply;
  const actions = usePovActions(reply?.requestId);
  const left = useCountdown(
    reply && !reply.saved && !reply.expired ? toMillis(reply.expiresAt) : 0,
  );
  if (!reply) return null;
  const asker = message.sender?.id !== me.id;
  const soft = tone === "own" ? "text-on-brand/80" : "text-ink-soft";

  return (
    <div className={`mt-1.5 flex items-center gap-2 text-[11px] font-bold ${soft}`}>
      <Camera className="size-3.5 shrink-0" />
      {reply.saved ? (
        <span className="flex items-center gap-1">
          <BookmarkCheck className="size-3.5" />
          {t("pov.saved")}
        </span>
      ) : (
        <span className="tabular-nums">
          {left > 0 ? t("pov.disappearsIn", { time: playTime(left / 1000) }) : t("pov.replyLabel")}
        </span>
      )}
      {asker && !reply.saved && !reply.expired && !message.id.startsWith("pending:") && (
        <button
          type="button"
          disabled={actions.busy}
          onClick={actions.save}
          className="ml-auto rounded-full bg-lime px-2.5 py-0.5 text-[#17122b]"
        >
          {t("pov.save")}
        </button>
      )}
    </div>
  );
}
