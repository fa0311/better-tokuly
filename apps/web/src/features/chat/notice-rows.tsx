import { useQueryClient } from "@tanstack/react-query";
import { chat, toMillis } from "@tokuly/api";
import { Phone, PhoneMissed, PhoneOff, Video } from "lucide-react";
import { Button } from "#/components/ui/button";
import { cn } from "#/lib/cn";
import { useTranslation } from "#/lib/use-translation";
import { joinCall, startCall, useCallSession } from "./calls/session";
import { useConversationContext } from "./conversation-context";
import { callText, systemText } from "./preview";
import { clock } from "./text";

/** Rows that are not anybody's bubble: what happened to the conversation, and its calls. */

/** "Aoi added Ren", centred between the bubbles. A pin notice leads to the pinned message. */
export function SystemLine({ message }: { message: chat.Message }) {
  const { me, jumpTo } = useConversationContext();
  const text = systemText(message.system, me.id);
  const pinned =
    message.system?.type === chat.SystemEventType.MESSAGE_PINNED ? message.system.messageId : "";
  const pill =
    "mx-auto block max-w-[85%] rounded-full bg-surface-2 px-3 py-1 text-center text-xs font-bold text-ink-soft";
  if (!text) return null;
  return pinned ? (
    <button
      type="button"
      onClick={() => jumpTo(pinned)}
      className={cn(pill, "hover:text-ink")}
      data-testid="system-message"
    >
      {text}
    </button>
  ) : (
    <p className={pill} data-testid="system-message">
      {text}
    </p>
  );
}

/** One call in the conversation: how it went, and the way back in (join while it runs, call back afterwards). */
export function CallCard({ message }: { message: chat.Message }) {
  const { t } = useTranslation("chat");
  const queryClient = useQueryClient();
  const { conversation, me, capabilities, writable, accepted } = useConversationContext();
  const inCall = useCallSession((state) => state.active?.callId);
  const call = message.call;
  if (!call) return null;
  const live = call.state !== chat.CallState.ENDED;
  const outgoing = call.startedById === me.id;
  const missed =
    !live &&
    !outgoing &&
    (call.endReason === chat.CallEndReason.MISSED ||
      call.endReason === chat.CallEndReason.CANCELLED);
  const unanswered = !live && call.endReason !== chat.CallEndReason.COMPLETED;
  const Icon = missed ? PhoneMissed : unanswered ? PhoneOff : call.video ? Video : Phone;
  const canCall = capabilities.callsEnabled && writable && accepted;

  return (
    <div
      className="mx-auto flex w-fit max-w-full items-center gap-3 rounded-full border-2 border-line bg-surface py-1.5 pr-2 pl-3"
      data-testid="call-message"
    >
      <span
        className={cn(
          "grid size-8 shrink-0 place-items-center rounded-full",
          live
            ? "bg-success text-white"
            : missed
              ? "bg-danger text-white"
              : "bg-surface-2 text-ink-soft",
        )}
      >
        <Icon className="size-4" />
      </span>
      <span className="min-w-0">
        <span className={cn("block truncate text-sm font-black", missed && "text-danger")}>
          {callText(call, me.id)}
        </span>
        <span className="block text-[11px] font-bold text-ink-faint">
          {clock(toMillis(message.createdAt))}
        </span>
      </span>
      {canCall && live && inCall !== call.callId && (
        <Button
          size="sm"
          variant="lime"
          onClick={() =>
            void joinCall(queryClient, {
              id: call.callId,
              conversationId: conversation.id,
              video: call.video,
            })
          }
        >
          {t("call.join")}
        </Button>
      )}
      {canCall && !live && (
        <Button
          size="sm"
          variant="ghost"
          className="px-3"
          onClick={() => void startCall(queryClient, conversation.id, call.video)}
        >
          {outgoing ? t("call.again") : t("call.back")}
        </Button>
      )}
    </div>
  );
}
