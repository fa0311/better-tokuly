import { useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { chat, common, errorReason } from "@tokuly/api";
import {
  type DragEvent,
  type HTMLAttributes,
  type ReactNode,
  useEffect,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { EmptyState } from "#/components/ui/page";
import { PaneSpinner } from "#/components/ui/spinner";
import { ReportDialog } from "#/features/engagement";
import { StampSheet } from "#/features/stamps";
import { cn } from "#/lib/cn";
import { realtime } from "#/lib/realtime";
import { useMe } from "#/stores/session";
import { useHideTabBar } from "#/stores/shell";
import { messageClient, useCapabilities } from "./api";
import { AssistConsent } from "./assist";
import { Composer } from "./composer";
import {
  ConversationBanners,
  ConversationHeader,
  JumpToLatest,
  RequestBar,
  TypingPill,
} from "./conversation-chrome";
import {
  type ConversationContextValue,
  ConversationProvider,
  type MediaTarget,
} from "./conversation-context";
import { type ConversationDialog, ConversationDialogs } from "./conversation-dialogs";
import { fetchConversation, useConversation } from "./conversations";
import { EmojiDialog } from "./emoji-picker";
import { errorMessage } from "./errors";
import { ForwardDialog } from "./forward-dialog";
import { InfoPanel, type PanelKind } from "./info-panel";
import { Lightbox, type LightboxItem } from "./lightbox";
import { useLive } from "./live";
import { LocationViewDialog } from "./location-dialog";
import { isVisual } from "./media-parts";
import { MessageList, type MessageListHandle } from "./message-list";
import { ReactionsSheet, ReceiptsSheet } from "./message-sheets";
import {
  catchUp,
  loadNewer,
  loadOlder,
  markRead,
  showMessage,
  showNewest,
  storeCreated,
  useMessageActions,
  useTimeline,
} from "./messages";
import { canWrite, isGroup, peerOf } from "./model";
import { previewMessage, send, useOutgoing } from "./outbox";
import { MediaPanel, PinnedPanel, SearchPanel, SidePanel } from "./panels";
import { displayName } from "./people";
import { PovRequestDialog } from "./pov";
import { PovCamera } from "./pov-camera";
import { contentText } from "./preview";
import { buildRows } from "./timeline";

/** A member who keeps a conversation open repeats that this often; the server forgets after 75 s. */
const PRESENCE_EVERY_MS = 30_000;

/** Whether the page is on screen (and not a background tab). */
function useVisible(): boolean {
  const [visible, setVisible] = useState(() => document.visibilityState === "visible");
  useEffect(() => {
    const update = () => setVisible(document.visibilityState === "visible");
    document.addEventListener("visibilitychange", update);
    return () => document.removeEventListener("visibilitychange", update);
  }, []);
  return visible;
}

/**
 * One conversation: its messages, the composer, and everything that can be
 * done in it. Full screen on phones (it covers the tab bar, like every chat
 * app), the right-hand pane on wide screens.
 */
export function ConversationScreen({
  conversationId,
  openAtMessageId,
}: {
  conversationId: string;
  openAtMessageId?: string;
}) {
  const { t } = useTranslation("chat");
  const navigate = useNavigate();
  const me = useMe()?.user;
  const conversation = useConversation(conversationId);
  const gone = conversation.isError || (conversation.isSuccess && !conversation.data);

  // The conversation is not (or no longer) the viewer's: left, removed, hidden or declined meanwhile.
  if (!me || conversation.isPending) return <ScreenFrame>{<PaneSpinner />}</ScreenFrame>;
  if (gone || !conversation.data) {
    return (
      <ScreenFrame>
        <div className="grid h-full place-items-center">
          <EmptyState
            emoji="🫥"
            title={t("gone.title")}
            body={t("gone.body")}
            action={
              <Button onClick={() => void navigate({ to: "/chat", search: {} })}>
                {t("gone.back")}
              </Button>
            }
          />
        </div>
      </ScreenFrame>
    );
  }
  return (
    <OpenConversation conversation={conversation.data} me={me} openAtMessageId={openAtMessageId} />
  );
}

function ScreenFrame({
  children,
  ...props
}: { children: ReactNode } & HTMLAttributes<HTMLDivElement>) {
  // A conversation takes the phone's whole height: its composer sits where the tab bar would be.
  useHideTabBar();
  return (
    <div className="flex h-full bg-bg" {...props}>
      {children}
    </div>
  );
}

function OpenConversation({
  conversation,
  me,
  openAtMessageId,
}: {
  conversation: chat.Conversation;
  me: common.UserSummary;
  openAtMessageId?: string;
}) {
  const { t } = useTranslation("chat");
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const capabilities = useCapabilities();
  const actions = useMessageActions();
  const visible = useVisible();
  const conversationId = conversation.id;
  const timeline = useTimeline(conversationId, openAtMessageId);
  const outgoing = useOutgoing(conversationId);
  const list = useRef<MessageListHandle>(null);

  const [replyTo, setReplyTo] = useState<chat.Message>();
  const [editing, setEditing] = useState<chat.Message>();
  const [panel, setPanel] = useState<PanelKind | null>(null);
  const [dialog, setDialog] = useState<ConversationDialog>(null);
  const [focus, setFocus] = useState<{ id?: string; nonce: number }>({
    id: openAtMessageId,
    nonce: 0,
  });
  const [atEnd, setAtEnd] = useState(true);
  const [arrived, setArrived] = useState(0);
  const [media, setMedia] = useState<{ items: LightboxItem[]; index: number | null }>({
    items: [],
    index: null,
  });
  const [stampId, setStampId] = useState<string>();
  const [place, setPlace] = useState<chat.LocationShare | null>(null);
  const [reacting, setReacting] = useState<chat.Message | null>(null);
  const [forwarding, setForwarding] = useState<chat.Message | null>(null);
  const [reporting, setReporting] = useState<chat.Message | null>(null);
  const [removing, setRemoving] = useState<chat.Message | null>(null);
  const [removingBusy, setRemovingBusy] = useState(false);
  const [reactionsOf, setReactionsOf] = useState<chat.Message | null>(null);
  const [receiptsOf, setReceiptsOf] = useState<chat.Message | null>(null);
  const [askingPov, setAskingPov] = useState(false);
  const [answeringPov, setAnsweringPov] = useState<string>();
  const [dropped, setDropped] = useState<File[]>([]);
  const [dragging, setDragging] = useState(false);
  /** Where "unread from here" goes: the viewer's read position when the conversation was opened (`null`: nothing was unread). */
  const [unreadAfter, setUnreadAfter] = useState<bigint | null>();

  const viewer = conversation.viewer;
  const accepted = viewer?.requestState === chat.RequestState.ACCEPTED;
  const pendingRequest = viewer?.requestState === chat.RequestState.PENDING;
  const writable = canWrite(conversation);
  const group = isGroup(conversation);
  const loaded = timeline.data;
  const messages = loaded?.messages ?? [];
  const hasNewer = loaded?.hasNewer ?? false;

  // ── Co-presence: "has the conversation open", repeated while it is on screen ──
  useEffect(() => {
    if (!visible) return;
    const client = realtime();
    const announce = () => client.sendConversationPresence(conversationId, true);
    announce();
    const timer = setInterval(announce, PRESENCE_EVERY_MS);
    // A new connection knows nothing of the old one's signals.
    const offFrame = client.onFrame((frame) => frame.body.case === "welcome" && announce());
    return () => {
      clearInterval(timer);
      offFrame();
      client.sendConversationPresence(conversationId, false);
    };
  }, [conversationId, visible]);

  // ── Reading ──
  // "Unread from here" marks where the viewer had read to when the conversation opened. It is settled
  // before the first messages are drawn, so the list can open on it.
  if (unreadAfter === undefined && loaded && viewer)
    setUnreadAfter(viewer.unreadCount > 0 ? viewer.readSeq : null);

  // "Read" means it was on screen: the read position follows the newest message the reader has actually
  // had in view while the page was visible, and moves on as they scroll.
  const [seen, setSeen] = useState<chat.Message>();
  const noteSeen = (message: chat.Message) =>
    setSeen((current) => (!current || message.seq > current.seq ? message : current));
  const readSeq = viewer?.readSeq;
  const seenId = seen?.id;
  const seenUpTo = seen?.seq;
  useEffect(() => {
    if (
      !visible ||
      !seenId ||
      seenUpTo === undefined ||
      readSeq === undefined ||
      seenUpTo <= readSeq
    )
      return;
    // A moment of scrolling is one request, not one per message passed.
    const timer = setTimeout(
      () => void markRead(queryClient, conversationId, seenId).catch(() => {}),
      150,
    );
    return () => clearTimeout(timer);
  }, [visible, seenId, seenUpTo, readSeq, conversationId, queryClient]);

  // While the reader is at the bottom of a visible page, messages that arrive are on screen at once:
  // they do not count as unread in the meantime.
  const reading = visible && atEnd && !hasNewer && loaded !== undefined;
  useEffect(() => {
    useLive.getState().setReading(reading ? conversationId : null);
    return () => useLive.getState().setReading(null);
  }, [reading, conversationId]);

  // ── New messages while scrolled away ──
  const newestSeq = messages.at(-1)?.seq;
  // A message that arrived while the first messages were still loading had nowhere to go: fetch it now.
  const missing = loaded !== undefined && !hasNewer && (newestSeq ?? 0n) < conversation.lastSeq;
  useEffect(() => {
    if (missing) void catchUp(queryClient, conversationId).catch(() => {});
  }, [missing, conversation.lastSeq, conversationId, queryClient]);
  const seenSeq = useRef(newestSeq);
  useEffect(() => {
    if (newestSeq === undefined) return;
    if (atEnd) {
      seenSeq.current = newestSeq;
      setArrived(0);
    } else if (seenSeq.current !== undefined && newestSeq > seenSeq.current) {
      setArrived(
        messages.filter((message) => message.seq > seenSeq.current! && message.sender?.id !== me.id)
          .length,
      );
    }
    // Counted when the newest message or the position changes.
  }, [newestSeq, atEnd]);

  // ── Moving around ──
  const jumpTo = (messageId: string) => {
    showMessage(queryClient, conversationId, messageId)
      .then(() => {
        setFocus((current) => ({ id: messageId, nonce: current.nonce + 1 }));
        setPanel((current) =>
          current && window.matchMedia("(min-width: 1180px)").matches ? current : null,
        );
      })
      .catch((error) =>
        toast.error(
          errorReason(error) === "MESSAGE_NOT_FOUND"
            ? t("message.notAvailable")
            : errorMessage(error),
        ),
      );
  };
  // The mark on a message that was jumped to fades by itself.
  useEffect(() => {
    if (!focus.id) return;
    const timer = setTimeout(() => setFocus((current) => ({ nonce: current.nonce })), 2200);
    return () => clearTimeout(timer);
  }, [focus.id, focus.nonce]);

  const toLatest = async () => {
    if (hasNewer)
      await showNewest(queryClient, conversationId).catch((error) =>
        toast.error(errorMessage(error)),
      );
    requestAnimationFrame(() => list.current?.scrollToEnd());
  };
  const afterSend = () => {
    if (hasNewer) void toLatest();
    else requestAnimationFrame(() => list.current?.scrollToEnd());
  };

  // ── Message actions that need the screen ──
  const openMedia = ({ message, index }: MediaTarget) => {
    const sender = displayName(message.sender);
    const items = message.attachments.filter(isVisual).map((attachment, position) => ({
      key: `${message.id}:${position}`,
      attachment,
      caption: [sender, message.text].filter(Boolean).join(" · "),
    }));
    setMedia({ items, index });
  };

  const confirmRemove = async () => {
    if (!removing) return;
    setRemovingBusy(true);
    try {
      await actions.remove(removing.id);
      setRemoving(null);
    } catch (error) {
      toast.error(errorMessage(error));
    } finally {
      setRemovingBusy(false);
    }
  };

  const sayHello = async () => {
    const peer = peerOf(conversation, me.id);
    if (!peer) return;
    try {
      const response = await messageClient().sendHello({ userId: peer.id });
      if (response.message) storeCreated(queryClient, response.message);
      void fetchConversation(queryClient, conversationId).catch(() => {});
    } catch (error) {
      // The two talked before (the history was cleared): a wave as an ordinary message does the same.
      if (errorReason(error) === "HELLO_NOT_FIRST")
        send(queryClient, conversationId, { text: "👋" });
      else toast.error(errorMessage(error));
    }
  };

  const context: ConversationContextValue = {
    conversation,
    me,
    capabilities,
    writable,
    accepted,
    reply: (message) => {
      setEditing(undefined);
      setReplyTo(message);
    },
    edit: (message) => {
      setReplyTo(undefined);
      setEditing(message);
    },
    forward: setForwarding,
    report: setReporting,
    remove: setRemoving,
    jumpTo,
    openMedia,
    openStamp: setStampId,
    showReactions: setReactionsOf,
    showReceipts: setReceiptsOf,
    pickReaction: setReacting,
    answerPov: setAnsweringPov,
  };

  const rows = buildRows(messages, {
    // A message already stored (its event beat the response) is not shown a second time as "sending".
    pending: outgoing
      .filter(
        (item) =>
          !messages.some(
            (message) => message.clientId === item.clientId && message.sender?.id === me.id,
          ),
      )
      .map((item) => previewMessage(item, me)),
    unreadAfterSeq: unreadAfter ?? undefined,
    myId: me.id,
  });
  const empty = loaded !== undefined && rows.length === 0;
  // A POV can be asked for between friends in a direct conversation; the server has the last word.
  const canAskPov = !group && writable && accepted;
  const requestPov = canAskPov ? () => setAskingPov(true) : undefined;

  const onDrop = (event: DragEvent) => {
    event.preventDefault();
    setDragging(false);
    const files = Array.from(event.dataTransfer.files);
    if (files.length > 0 && writable && !pendingRequest) setDropped(files);
  };

  return (
    <ConversationProvider value={context}>
      <ScreenFrame data-testid="conversation-screen">
        <div
          className="relative flex h-full min-w-0 flex-1 flex-col"
          onDragOver={(event) => {
            if (!event.dataTransfer.types.includes("Files")) return;
            event.preventDefault();
            setDragging(true);
          }}
          onDragLeave={(event) => event.currentTarget === event.target && setDragging(false)}
          onDrop={onDrop}
        >
          <ConversationHeader onPanel={setPanel} onDialog={setDialog} onRequestPov={requestPov} />
          <ConversationBanners onPanel={setPanel} onJump={jumpTo} />

          <div className="relative min-h-0 flex-1">
            {timeline.isPending ? (
              <PaneSpinner />
            ) : timeline.isError ? (
              <div className="grid h-full place-items-center">
                <EmptyState
                  emoji="😵‍💫"
                  title={errorMessage(timeline.error)}
                  action={
                    <Button onClick={() => void timeline.refetch()}>{t("actions.retry")}</Button>
                  }
                />
              </div>
            ) : empty ? (
              <div className="grid h-full place-items-center" data-testid="empty-conversation">
                {group ? (
                  <EmptyState
                    emoji="🎉"
                    title={t("empty.group.title")}
                    body={t("empty.group.body")}
                  />
                ) : (
                  <EmptyState
                    emoji="👋"
                    title={t("empty.direct.title", {
                      name: displayName(peerOf(conversation, me.id)),
                    })}
                    body={t("empty.direct.body")}
                    action={
                      writable && (
                        <Button variant="lime" size="lg" onClick={() => void sayHello()}>
                          {t("hello.wave")}
                        </Button>
                      )
                    }
                  />
                )}
              </div>
            ) : (
              <MessageList
                ref={list}
                rows={rows}
                hasOlder={loaded?.hasOlder ?? false}
                hasNewer={hasNewer}
                focusId={focus.id}
                focusNonce={focus.nonce}
                onNearStart={() => void loadOlder(queryClient, conversationId).catch(() => {})}
                onNearEnd={() => void loadNewer(queryClient, conversationId).catch(() => {})}
                onAtEndChange={setAtEnd}
                onSeen={noteSeen}
                onOpenLocation={setPlace}
              />
            )}
            <TypingPill />
            {!atEnd && !empty && <JumpToLatest count={arrived} onClick={() => void toLatest()} />}
          </div>

          {pendingRequest ? (
            <RequestBar onDeclined={() => void navigate({ to: "/chat", search: {} })} />
          ) : (
            <Composer
              replyTo={replyTo}
              onCancelReply={() => setReplyTo(undefined)}
              editing={editing}
              onCancelEdit={() => setEditing(undefined)}
              onSent={afterSend}
              onRequestPov={requestPov}
              dropped={dropped}
            />
          )}

          <div
            className={cn(
              "pointer-events-none absolute inset-2 z-30 grid place-items-center rounded-card border-4 border-dashed border-brand bg-brand-soft/80 text-lg font-black text-brand transition-opacity",
              dragging ? "opacity-100" : "opacity-0",
            )}
          >
            {t("composer.dropHere")}
          </div>
        </div>

        {panel && (
          <SidePanel title={t(`panel.${panel}`)} onClose={() => setPanel(null)}>
            {panel === "info" && <InfoPanel onShow={setPanel} onDialog={setDialog} />}
            {panel === "search" && <SearchPanel onOpen={jumpTo} />}
            {panel === "pinned" && <PinnedPanel onOpen={jumpTo} />}
            {panel === "media" && <MediaPanel onOpen={jumpTo} />}
          </SidePanel>
        )}

        <Lightbox
          items={media.items}
          index={media.index}
          onIndexChange={(index) => setMedia((current) => ({ ...current, index }))}
          onClose={() => setMedia((current) => ({ ...current, index: null }))}
        />
        <StampSheet stampId={stampId} onClose={() => setStampId(undefined)} />
        <LocationViewDialog location={place} onClose={() => setPlace(null)} />
        <EmojiDialog
          open={reacting !== null}
          onOpenChange={(open) => !open && setReacting(null)}
          title={t("message.react")}
          onPick={(emoji) => {
            if (!reacting) return;
            actions.react(reacting, emoji).catch((error) => toast.error(errorMessage(error)));
          }}
        />
        <ForwardDialog message={forwarding} onClose={() => setForwarding(null)} />
        {reporting && (
          <ReportDialog
            subject={{ type: common.SubjectType.CHAT_MESSAGE, id: reporting.id }}
            open
            onOpenChange={(open) => !open && setReporting(null)}
            label={`${displayName(reporting.sender)}: ${contentText(reporting, me.id)}`}
          />
        )}
        <ReactionsSheet message={reactionsOf} onClose={() => setReactionsOf(null)} />
        <ReceiptsSheet message={receiptsOf} onClose={() => setReceiptsOf(null)} />
        <ConfirmDialog
          open={removing !== null}
          onOpenChange={(open) => !open && setRemoving(null)}
          title={t("message.deleteTitle")}
          description={
            removing && removing.sender?.id !== me.id
              ? t("message.deleteOthersLead")
              : t("message.deleteLead")
          }
          confirmLabel={t("message.delete")}
          danger
          loading={removingBusy}
          onConfirm={() => void confirmRemove()}
        />
        {canAskPov && <PovRequestDialog open={askingPov} onOpenChange={setAskingPov} />}
        <PovCamera
          open={answeringPov !== undefined}
          onClose={() => setAnsweringPov(undefined)}
          requestId={answeringPov ?? ""}
          onSent={afterSend}
        />
        <ConversationDialogs
          dialog={dialog}
          myId={me.id}
          onClose={() => setDialog(null)}
          onGone={() => void navigate({ to: "/chat", search: {} })}
        />
        <AssistConsent />
      </ScreenFrame>
    </ConversationProvider>
  );
}
