import { useQueryClient } from "@tanstack/react-query";
import { type chat, toMillis } from "@tokuly/api";
import { isGroup, ringsFor } from "@tokuly/core/chat";
import { Phone, PhoneOff, Video } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Avatar } from "#/components/ui/avatar";
import { EmojiText } from "#/features/emoji";
import { useMe } from "#/stores/session";
import { useConversation } from "../conversations";
import { useLive } from "../live";
import { displayName, titleOf } from "../people";
import { CallScreen } from "./call-screen";
import { ring } from "./ringtone";
import { declineCall, joinCall, useCallSession } from "./session";

/**
 * Calls, wherever the person is in the app: the sheet that rings for an
 * incoming call and the screen of the call they are in. Mounted once for the
 * signed-in app.
 */
export function CallHost() {
  const myId = useMe()?.user?.id ?? "";
  const calls = useLive((state) => state.calls);
  const active = useCallSession((state) => state.active);
  // One call rings at a time: the one that started first.
  const ringing = Object.values(calls)
    .filter((call) => ringsFor(call, myId) && call.id !== active?.callId)
    .sort((a, b) => toMillis(a.startedAt) - toMillis(b.startedAt))[0];

  return (
    <>
      {ringing && <IncomingCall key={ringing.id} call={ringing} myId={myId} />}
      {active && <CallScreen active={active} />}
    </>
  );
}

function IncomingCall({ call, myId }: { call: chat.Call; myId: string }) {
  const { t } = useTranslation("chat");
  const queryClient = useQueryClient();
  const conversation = useConversation(call.conversationId).data;
  const [answered, setAnswered] = useState(false);
  // Should the "stopped ringing" event get lost, the ringing still ends on time.
  const [expired, setExpired] = useState(false);
  const ringUntil = toMillis(call.ringExpiresAt);
  useEffect(() => {
    if (!ringUntil) return;
    const timer = setTimeout(() => setExpired(true), Math.max(0, ringUntil - Date.now()));
    return () => clearTimeout(timer);
  }, [ringUntil]);

  const showing = !answered && !expired;
  useEffect(() => (showing ? ring("incoming") : undefined), [showing]);
  if (!showing) return null;

  const caller = call.startedBy;
  const group = conversation && isGroup(conversation);
  return (
    <div
      role="alertdialog"
      aria-label={t("call.incoming.label", { name: displayName(caller) })}
      className="pointer-events-none fixed inset-x-0 top-0 z-[80] flex justify-center px-3 pt-safe"
      data-testid="incoming-call"
    >
      <div className="sticker-lg pointer-events-auto flex w-full max-w-sm animate-slide-up items-center gap-3 rounded-[2rem] bg-surface p-3">
        <span className="relative shrink-0">
          <span className="absolute inset-0 animate-pulse-ring rounded-[0.85rem] bg-success" />
          <Avatar user={caller} outlined className="relative" />
        </span>
        <div className="min-w-0 flex-1">
          <p className="truncate font-black">
            <EmojiText text={displayName(caller)} />
          </p>
          <p className="truncate text-sm font-bold text-ink-soft">
            {group
              ? t(call.video ? "call.incoming.groupVideo" : "call.incoming.groupVoice", {
                  group: titleOf(conversation, myId),
                })
              : t(call.video ? "call.incoming.video" : "call.incoming.voice")}
          </p>
        </div>
        <button
          type="button"
          aria-label={t("call.decline")}
          onClick={() => {
            setAnswered(true);
            void declineCall(call.id);
          }}
          className="pressable sticker-sm grid size-12 shrink-0 place-items-center rounded-full bg-danger text-white"
        >
          <PhoneOff className="size-5" />
        </button>
        <button
          type="button"
          aria-label={t("call.accept")}
          onClick={() => {
            setAnswered(true);
            void joinCall(queryClient, call);
          }}
          className="pressable sticker-sm grid size-12 shrink-0 place-items-center rounded-full bg-success text-white"
        >
          {call.video ? <Video className="size-5" /> : <Phone className="size-5" />}
        </button>
      </div>
    </div>
  );
}
