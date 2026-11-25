import { useInfiniteQuery, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { chat, toMillis } from "@tokuly/api";
import { participantState } from "@tokuly/core/chat";
import { ChevronLeft, Phone, PhoneIncoming, PhoneMissed, PhoneOutgoing, Video } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { Tabs } from "#/components/ui/tabs";
import { cn } from "#/lib/cn";
import { useMe } from "#/stores/session";
import { callClient, chatKeys, useCapabilities } from "../api";
import { readConversation } from "../conversations";
import { displayName, titleOf } from "../people";
import { dateTime, playTime } from "../text";
import { joinCall, startCall } from "./session";

type Filter = "all" | "missed";

/** Whether the viewer did not pick this call up. */
function missedBy(call: chat.Call, myId: string): boolean {
  if (call.startedBy?.id === myId) return false;
  const state = participantState(call, myId);
  return (
    state === chat.CallParticipantState.MISSED ||
    (call.state === chat.CallState.ENDED && state === chat.CallParticipantState.RINGING)
  );
}

/** The viewer's calls across all conversations, newest first; "missed" narrows it to the ones they did not answer. */
export function CallHistoryScreen() {
  const { t } = useTranslation("chat");
  const myId = useMe()?.user?.id ?? "";
  const [filter, setFilter] = useState<Filter>("all");
  const missedOnly = filter === "missed";
  const calls = useInfiniteQuery({
    queryKey: chatKeys.calls(missedOnly),
    queryFn: ({ pageParam }) =>
      callClient().listCalls({ missedOnly, page: { cursor: pageParam, limit: 30 } }),
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
  });
  const rows = calls.data?.pages.flatMap((page) => page.calls) ?? [];

  return (
    <div className="fixed inset-0 z-40 flex flex-col bg-bg md:static md:z-auto md:h-full">
      <header className="glass flex h-16 shrink-0 items-center gap-2 border-b-2 border-line px-3">
        <Link
          to="/chat"
          aria-label={t("actions.back")}
          className="grid size-9 place-items-center rounded-full hover:bg-surface-2 md:hidden"
        >
          <ChevronLeft className="size-6" />
        </Link>
        <h1 className="min-w-0 flex-1 truncate px-1 text-xl font-black">{t("calls.history")}</h1>
      </header>
      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="mx-auto max-w-2xl px-3 pt-4 pb-10">
          <Tabs<Filter>
            className="mb-3"
            value={filter}
            onChange={setFilter}
            items={[
              { value: "all", label: t("calls.all") },
              { value: "missed", label: t("calls.missed") },
            ]}
          />
          {calls.isPending ? (
            <div className="flex flex-col gap-2">
              {[0, 1, 2].map((index) => (
                <Skeleton key={index} className="h-16" />
              ))}
            </div>
          ) : rows.length === 0 ? (
            <EmptyState
              emoji={missedOnly ? "🙌" : "📞"}
              title={missedOnly ? t("calls.emptyMissed") : t("calls.empty")}
              body={missedOnly ? undefined : t("calls.emptyBody")}
            />
          ) : (
            <ul className="flex flex-col">
              {rows.map((call) => (
                <li key={call.id}>
                  <CallRow call={call} myId={myId} />
                </li>
              ))}
            </ul>
          )}
          <LoadMore query={calls} />
        </div>
      </div>
    </div>
  );
}

function CallRow({ call, myId }: { call: chat.Call; myId: string }) {
  const { t } = useTranslation("chat");
  const queryClient = useQueryClient();
  const capabilities = useCapabilities();
  const outgoing = call.startedBy?.id === myId;
  const missed = missedBy(call, myId);
  const live = call.state !== chat.CallState.ENDED;
  const conversation = readConversation(queryClient, call.conversationId);
  // A group call is known by its group; a direct one by the other person.
  const others = call.participants.flatMap((participant) =>
    participant.user && participant.user.id !== myId ? [participant.user] : [],
  );
  const face = outgoing ? others[0] : call.startedBy;
  const name = conversation
    ? titleOf(conversation, myId)
    : others.length > 1
      ? others.map(displayName).join(t("people.separator"))
      : displayName(face);
  const Icon = missed ? PhoneMissed : outgoing ? PhoneOutgoing : PhoneIncoming;
  const outcome = live
    ? t("calls.live")
    : missed
      ? t("call.summary.missed")
      : call.durationSeconds > 0
        ? playTime(call.durationSeconds)
        : outgoing
          ? t(
              call.endReason === chat.CallEndReason.DECLINED
                ? "call.summary.declinedByThem"
                : call.endReason === chat.CallEndReason.CANCELLED
                  ? "call.summary.cancelled"
                  : "call.summary.noAnswer",
            )
          : t("call.summary.declined");

  return (
    <div
      className="flex items-center gap-3 rounded-2xl px-3 py-2.5 hover:bg-surface-2"
      data-testid="call-row"
    >
      <Link
        to="/chat/$conversationId"
        params={{ conversationId: call.conversationId }}
        className="flex min-w-0 flex-1 items-center gap-3"
      >
        <Avatar user={face} outlined />
        <span className="min-w-0 flex-1">
          <span className={cn("block truncate font-black", missed && "text-danger")}>{name}</span>
          <span className="flex items-center gap-1.5 text-sm text-ink-soft">
            <Icon className={cn("size-4 shrink-0", missed && "text-danger")} />
            {call.video && <Video className="size-4 shrink-0" />}
            <span className="truncate">
              {outcome} · {dateTime(toMillis(call.startedAt))}
            </span>
          </span>
        </span>
      </Link>
      {capabilities.callsEnabled &&
        (live ? (
          <Button size="sm" variant="lime" onClick={() => void joinCall(queryClient, call)}>
            {t("call.join")}
          </Button>
        ) : (
          <Button
            size="icon-sm"
            variant="ghost"
            aria-label={t("call.callBack", { name })}
            onClick={() => void startCall(queryClient, call.conversationId, call.video)}
          >
            {call.video ? <Video className="size-5" /> : <Phone className="size-5" />}
          </Button>
        ))}
    </div>
  );
}
