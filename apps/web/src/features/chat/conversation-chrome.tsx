import { useInfiniteQuery, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { type chat, toMillis } from "@tokuly/api";
import { Camera, ChevronDown, ChevronLeft, Eye, Images, MoreVertical, Phone, Pin, Search, Video } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Avatar } from "#/components/ui/avatar";
import { CountBadge } from "#/components/ui/badge";
import { Button } from "#/components/ui/button";
import { Menu, type MenuAction } from "#/components/ui/menu";
import { cn } from "#/lib/cn";
import { useConversationActions } from "./actions";
import { chatKeys, messageClient } from "./api";
import { joinCall, startCall, useCallSession } from "./calls/session";
import { ConversationAvatar } from "./conversation-avatar";
import { useConversationContext } from "./conversation-context";
import type { ConversationDialog } from "./conversation-dialogs";
import { useConversationMenu } from "./conversation-menu";
import type { PanelKind } from "./info-panel";
import { useLive, useLiveCall, usePresent, useTyping } from "./live";
import { isGroup, joinedParticipants, peerOf } from "./model";
import { displayName, titleOf, useOnline } from "./people";
import { useCountdown } from "./pov";
import { contentText } from "./preview";
import { playTime } from "./text";

/** The parts of an open conversation around its messages: the header, the notices under it, and the small overlays. */

function namesOf(conversation: chat.Conversation, userIds: string[]): string {
  const names = userIds.map((userId) => {
    const member = conversation.members.find((candidate) => candidate.id === userId);
    return member ? displayName(member) : "";
  });
  return names.filter(Boolean).join("、");
}

export function ConversationHeader({ onPanel, onDialog, onRequestPov }: { onPanel(panel: PanelKind): void; onDialog(dialog: ConversationDialog): void; onRequestPov?(): void }) {
  const { t } = useTranslation("chat");
  const queryClient = useQueryClient();
  const { conversation, me, capabilities, writable, accepted } = useConversationContext();
  const group = isGroup(conversation);
  const peer = peerOf(conversation, me.id);
  const online = useOnline(peer?.id);
  const typing = useTyping(conversation.id, me.id);
  const present = usePresent(conversation.id, me.id);
  const inCall = useCallSession((state) => Boolean(state.active));
  const conversationMenu = useConversationMenu(conversation, onDialog);
  const title = titleOf(conversation, me.id);
  const canCall = capabilities.callsEnabled && writable && accepted && !conversation.activeCallId && !inCall;

  const subtitle =
    typing.length > 0
      ? { text: group ? t("typing.named", { names: namesOf(conversation, typing) || t("people.someone") }) : t("typing.short"), className: "text-brand" }
      : present.length > 0
        ? { text: group ? t("presence.group", { names: namesOf(conversation, present) || t("people.someone") }) : t("presence.direct"), className: "text-success" }
        : group
          ? { text: t("group.memberCount", { count: conversation.memberCount }), className: "text-ink-soft" }
          : online && !conversation.readOnly
            ? { text: t("presence.online"), className: "text-success" }
            : { text: peer && !peer.deleted ? `@${peer.handle}` : "", className: "text-ink-soft" };

  const menu: MenuAction[] = [
    { label: t("info.search"), icon: <Search />, onSelect: () => onPanel("search") },
    { label: t("info.pins"), icon: <Pin />, onSelect: () => onPanel("pinned") },
    { label: t("info.media"), icon: <Images />, onSelect: () => onPanel("media") },
    ...(onRequestPov ? [{ label: t("composer.pov"), icon: <Camera />, onSelect: onRequestPov }] : []),
    ...conversationMenu,
  ];
  const round = "grid size-10 shrink-0 place-items-center rounded-full text-ink-soft hover:bg-surface-2 hover:text-ink";

  return (
    <header className="glass z-20 flex h-16 shrink-0 items-center gap-1 border-b-2 border-line px-2">
      <Link to="/chat" aria-label={t("actions.back")} className={cn(round, "md:hidden")}>
        <ChevronLeft className="size-6" />
      </Link>
      <button type="button" onClick={() => onPanel("info")} aria-label={t("info.open", { name: title })} className="flex min-w-0 flex-1 items-center gap-3 rounded-2xl px-1.5 py-1 text-left hover:bg-surface-2 md:ml-1">
        <ConversationAvatar conversation={conversation} myId={me.id} />
        <span className="min-w-0 flex-1">
          <span className="block truncate font-black" data-testid="conversation-title">
            {title}
          </span>
          <span className={cn("flex items-center gap-1 truncate text-xs font-bold", subtitle.className)} data-testid="conversation-subtitle">
            {present.length > 0 && typing.length === 0 && <Eye className="size-3.5 shrink-0" />}
            <span className="truncate">{subtitle.text}</span>
          </span>
        </span>
      </button>
      {canCall && (
        <>
          <button type="button" aria-label={t("call.startVoice")} title={t("call.startVoice")} className={round} onClick={() => void startCall(queryClient, conversation.id, false)}>
            <Phone className="size-5" />
          </button>
          <button type="button" aria-label={t("call.startVideo")} title={t("call.startVideo")} className={round} onClick={() => void startCall(queryClient, conversation.id, true)}>
            <Video className="size-5" />
          </button>
        </>
      )}
      <Menu
        actions={menu}
        trigger={
          <button type="button" aria-label={t("menu.conversation")} className={round}>
            <MoreVertical className="size-5" />
          </button>
        }
      />
    </header>
  );
}

/** A call is running here and the viewer is not in it: who is, and the way in. */
function CallBanner() {
  const { t } = useTranslation("chat");
  const queryClient = useQueryClient();
  const { conversation, capabilities, accepted } = useConversationContext();
  const call = useLiveCall(conversation.activeCallId);
  const mine = useCallSession((state) => state.active?.callId);
  if (!call || !capabilities.callsEnabled || !accepted || mine === call.id) return null;
  const joined = joinedParticipants(call);
  return (
    <div className="flex items-center gap-3 border-b-2 border-line bg-success/15 px-4 py-2" data-testid="call-banner">
      <span className="grid size-8 shrink-0 place-items-center rounded-full bg-success text-white">{call.video ? <Video className="size-4" /> : <Phone className="size-4" />}</span>
      <p className="min-w-0 flex-1 truncate text-sm font-black">{joined.length > 0 ? t("call.banner.running", { count: joined.length }) : t("call.banner.ringing")}</p>
      <span className="flex -space-x-2">
        {joined.slice(0, 3).map((user) => (
          <Avatar key={user.id} user={user} size="xs" />
        ))}
      </span>
      <Button size="sm" variant="lime" onClick={() => void joinCall(queryClient, call)}>
        {t("call.join")}
      </Button>
    </div>
  );
}

/** A POV request is waiting: how long is left, and the way to it. */
function PovBanner({ onOpen }: { onOpen(messageId: string): void }) {
  const { t } = useTranslation("chat");
  const { conversation, me } = useConversationContext();
  const request = useLive((state) => state.openPov[conversation.id]);
  const left = useCountdown(request ? toMillis(request.expiresAt) : 0);
  if (!request || left <= 0) return null;
  const mine = request.requesterId === me.id;
  return (
    <button type="button" onClick={() => onOpen(request.requestMessageId)} className="flex w-full items-center gap-3 border-b-2 border-line bg-sun/25 px-4 py-2 text-left" data-testid="pov-banner">
      <span className="grid size-8 shrink-0 place-items-center rounded-full bg-sun text-[#17122b]">
        <Camera className="size-4" />
      </span>
      <span className="min-w-0 flex-1 truncate text-sm font-black">{mine ? t("pov.banner.waiting") : t("pov.banner.asked")}</span>
      <span className="shrink-0 text-xs font-bold text-ink-soft tabular-nums">{t("pov.timeLeft", { time: playTime(left / 1000) })}</span>
    </button>
  );
}

/** The most recently pinned message; opens the list of pins. */
function PinnedBar({ onOpen }: { onOpen(): void }) {
  const { conversation, me } = useConversationContext();
  const pinned = useInfiniteQuery({
    queryKey: chatKeys.pinned(conversation.id),
    queryFn: ({ pageParam }) => messageClient().listPinnedMessages({ conversationId: conversation.id, page: { cursor: pageParam, limit: 30 } }),
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
  });
  const messages = pinned.data?.pages.flatMap((page) => page.messages) ?? [];
  const latest = messages[0];
  if (!latest) return null;
  return (
    <button type="button" onClick={onOpen} className="flex w-full items-center gap-2 border-b-2 border-line bg-surface px-4 py-2 text-left" data-testid="pinned-bar">
      <Pin className="size-4 shrink-0 text-brand" />
      <span className="min-w-0 flex-1 truncate text-sm">
        <span className="font-black">{displayName(latest.sender)}</span> <span className="text-ink-soft">{contentText(latest, me.id)}</span>
      </span>
      {messages.length > 1 && <span className="shrink-0 rounded-full bg-surface-2 px-2 text-xs font-bold text-ink-soft">{messages.length}</span>}
    </button>
  );
}

/** The notices between the header and the messages. */
export function ConversationBanners({ onPanel, onJump }: { onPanel(panel: PanelKind): void; onJump(messageId: string): void }) {
  return (
    <>
      <CallBanner />
      <PovBanner onOpen={onJump} />
      <PinnedBar onOpen={() => onPanel("pinned")} />
    </>
  );
}

/** In place of the composer while a message request waits for the viewer's answer. */
export function RequestBar({ onDeclined }: { onDeclined(): void }) {
  const { t } = useTranslation("chat");
  const { conversation, me } = useConversationContext();
  const actions = useConversationActions();
  const peer = peerOf(conversation, me.id);
  return (
    <div className="shrink-0 border-t-2 border-line bg-surface px-4 pt-3 pb-safe text-center" data-testid="request-bar">
      <p className="font-black">{t("request.title", { name: displayName(peer) })}</p>
      <p className="mt-0.5 text-sm text-ink-soft">{t("request.lead")}</p>
      <div className="mt-3 flex justify-center gap-2">
        <Button variant="ghost" onClick={() => void actions.respond(conversation, false).then((done) => done && onDeclined())}>
          {t("request.decline")}
        </Button>
        <Button variant="lime" onClick={() => void actions.respond(conversation, true)}>
          {t("request.accept")}
        </Button>
      </div>
    </div>
  );
}

/** "Aoi is typing…" over the bottom of the list (it takes no room, so nothing jumps). */
export function TypingPill() {
  const { t } = useTranslation("chat");
  const { conversation, me } = useConversationContext();
  const typing = useTyping(conversation.id, me.id);
  if (typing.length === 0) return null;
  const first = conversation.members.find((member) => member.id === typing[0]);
  return (
    <div className="pointer-events-none absolute bottom-2 left-3 z-10 flex animate-pop-in items-center gap-2 rounded-full border-2 border-line bg-surface py-1 pr-3 pl-1.5" role="status" data-testid="typing-indicator">
      {first && <Avatar user={first} size="xs" />}
      <span className="flex items-center gap-1" aria-hidden>
        {[0, 1, 2].map((dot) => (
          <span key={dot} className="size-1.5 animate-bounce rounded-full bg-ink-faint" style={{ animationDelay: `${dot * 140}ms` }} />
        ))}
      </span>
      <span className="sr-only">{t("typing.short")}</span>
    </div>
  );
}

/** Appears while the reader is away from the newest messages; counts what arrived meanwhile. */
export function JumpToLatest({ count, onClick }: { count: number; onClick(): void }) {
  const { t } = useTranslation("chat");
  return (
    <button type="button" onClick={onClick} aria-label={t("message.jumpToLatest")} className="pressable sticker-sm absolute right-3 bottom-3 z-10 grid size-11 animate-pop-in place-items-center rounded-full bg-surface" data-testid="jump-to-latest">
      <ChevronDown className="size-6" />
      <CountBadge count={count} className="absolute -top-2 -right-1" />
    </button>
  );
}
