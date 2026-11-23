import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { common, community, errorReason } from "@tokuly/api";
import {
  ArrowDownUp,
  Bell,
  BellOff,
  CornerDownRight,
  Eye,
  History,
  Lock,
  Megaphone,
  MoreHorizontal,
  Pencil,
  Pin,
  Reply as ReplyIcon,
  SquareCheckBig,
  Trash2,
  Unlock,
} from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Chip } from "#/components/ui/card";
import { ConfirmDialog } from "#/components/ui/confirm";
import { Dialog } from "#/components/ui/dialog";
import { TextField } from "#/components/ui/field";
import { LoadMore } from "#/components/ui/load-more";
import { Menu, type MenuAction } from "#/components/ui/menu";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { EmojiText } from "#/features/emoji";
import { ReactionBar } from "#/features/engagement";
import { StampImage, StampSheet } from "#/features/stamps";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import {
  boardClient,
  keys,
  NS,
  patchReplies,
  patchThread,
  upsertReply,
  useChannels,
  useReplies,
  useThread,
} from "./api";
import { PostTextArea, ReplyComposer } from "./composer";
import { useCurrentCommunity } from "./context";
import { ContentHistoryDialog, ReasonDialog, RevealAuthorDialog } from "./mod-dialogs";
import {
  can,
  communityHref,
  isMember,
  parseTags,
  pseudonymName,
  threadHref,
  timeoutUntilMs,
} from "./model";
import { Notice, PostAuthor, PostBody, PostImages, TagList } from "./parts";
import { PollCard } from "./poll";
import { useShareMenu } from "./share";
import { ThreadFlags } from "./thread-card";

const P = community.Permission;

/** A thread with its poll, its replies and the box to answer. */
export function ThreadScreen({ threadId }: { threadId: string }) {
  const { t } = useTranslation(NS);
  const c = useCurrentCommunity();
  const queryClient = useQueryClient();
  const query = useThread(threadId);
  const thread = query.data;
  const back = communityHref(c.handle);
  const member = isMember(c);
  const threadUuid = thread?.id;
  const unread = thread?.viewer?.unread ?? false;

  // Opening a thread reads it.
  useEffect(() => {
    if (!threadUuid || !member || !unread) return;
    boardClient()
      .markRead({ target: { case: "threadId", value: threadUuid } })
      .then(() => {
        patchThread(queryClient, threadUuid, (each) =>
          each.viewer
            ? { ...each, viewer: { ...each.viewer, unread: false, unreadReplyCount: 0 } }
            : each,
        );
        void queryClient.invalidateQueries({ queryKey: keys.channels(c.id) });
        void queryClient.invalidateQueries({ queryKey: keys.communities });
        void queryClient.invalidateQueries({ queryKey: keys.my });
      })
      .catch(() => {});
  }, [threadUuid, member, unread, queryClient, c.id]);

  if (query.isPending)
    return (
      <Page title={c.name} back={back}>
        <Skeleton className="h-64" />
      </Page>
    );
  if (!thread || thread.communityId !== c.id) {
    const reason = query.error ? errorReason(query.error) : "THREAD_NOT_FOUND";
    return (
      <Page title={c.name} back={back}>
        <EmptyState
          emoji="🫥"
          title={
            !reason || reason === "THREAD_NOT_FOUND"
              ? t("thread.gone")
              : errorMessage(query.error, { namespace: NS })
          }
          body={t("thread.goneBody")}
        />
      </Page>
    );
  }
  return (
    <Page title={<EmojiText text={c.name} />} back={back}>
      <ThreadBody community={c} thread={thread} />
    </Page>
  );
}

function ThreadBody({
  community: c,
  thread,
}: {
  community: community.Community;
  thread: community.Thread;
}) {
  const { t } = useTranslation(NS);
  const [newestFirst, setNewestFirst] = useState(false);
  const [replyTo, setReplyTo] = useState<community.Reply>();
  const [stampId, setStampId] = useState<string>();
  const channels = useChannels(c.id);
  const channel = channels.data?.find((each) => each.id === thread.channelId);
  const replies = useReplies(thread.id, newestFirst);
  const viewer = thread.viewer;
  const timedOut = timeoutUntilMs(c.viewer, Date.now()) > 0;
  const subject = { type: common.SubjectType.COMMUNITY_THREAD, id: thread.id };

  return (
    <>
      <article className="sticker rounded-card bg-surface p-4 sm:p-5">
        <div className="flex items-start gap-2">
          <div className="min-w-0 flex-1">
            <PostAuthor
              author={thread.author}
              pseudonym={thread.pseudonym}
              mine={viewer?.mine}
              createdAtMs={Number(thread.createdAt?.seconds ?? 0n) * 1000}
              edited={Boolean(thread.editedAt)}
            />
          </div>
          <ThreadActions community={c} thread={thread} />
        </div>
        <div className="mt-3 flex flex-wrap gap-1.5">
          {channel && (
            <Link to="/g/$handle" params={{ handle: c.handle }} search={{ channel: channel.id }}>
              <Chip>
                {channel.emoji} {channel.name}
              </Chip>
            </Link>
          )}
          <ThreadFlags thread={thread} />
        </div>
        <h2 className="mt-2 break-words text-2xl font-black leading-snug">
          <EmojiText text={thread.title} />
        </h2>
        {thread.state === community.PostState.HIDDEN && (
          <Notice tone="warn" className="mt-3">
            {thread.hiddenReason === community.HiddenReason.MUTED_WORD
              ? t("hidden.mutedWord")
              : t("hidden.reports")}
          </Notice>
        )}
        <PostBody body={thread.body} mentions={thread.mentions} className="mt-3 text-[16px]" />
        {thread.images.length > 0 && (
          <div className="mt-3">
            <PostImages images={thread.images} />
          </div>
        )}
        {thread.stamp && (
          <button
            type="button"
            className="mt-3 block"
            onClick={() => setStampId(thread.stamp?.id)}
            aria-label={thread.stamp.name || t("compose.stamp")}
          >
            <StampImage stamp={thread.stamp} size={128} />
          </button>
        )}
        {thread.poll && (
          <div className="mt-4">
            <PollCard thread={thread} canVote={isMember(c) && !timedOut} />
          </div>
        )}
        <TagList tags={thread.tags} className="mt-3" />
        <div className="mt-4 flex flex-wrap items-center gap-2">
          <ReactionBar subject={subject} className="min-w-0 flex-1" />
          {isMember(c) && <FollowButton thread={thread} />}
        </div>
      </article>

      <div className="mt-6 mb-2 flex items-center gap-2 px-1">
        <h3 className="flex-1 text-base font-black">
          {t("thread.replies", { count: thread.replyCount })}
        </h3>
        <Button
          variant="ghost"
          size="sm"
          icon={<ArrowDownUp className="size-4" />}
          onClick={() => setNewestFirst(!newestFirst)}
        >
          {newestFirst ? t("reply.newest") : t("reply.oldest")}
        </Button>
      </div>

      {replies.isPending ? (
        <Skeleton className="h-24" />
      ) : (replies.items?.length ?? 0) === 0 ? (
        <p className="px-1 py-6 text-center text-sm text-ink-faint">{t("reply.empty")}</p>
      ) : (
        <ol className="flex flex-col gap-2">
          {replies.items?.map((reply) => (
            <li key={reply.id}>
              <ReplyItem
                community={c}
                thread={thread}
                reply={reply}
                onReplyTo={viewer?.canReply ? () => setReplyTo(reply) : undefined}
                onOpenStamp={setStampId}
              />
            </li>
          ))}
          <LoadMore query={replies} />
        </ol>
      )}

      {viewer?.canReply ? (
        <ReplyComposer
          community={c}
          thread={thread}
          channel={channel}
          replyTo={replyTo}
          onClearReplyTo={() => setReplyTo(undefined)}
        />
      ) : (
        <Notice className="mt-4" icon={thread.locked ? <Lock /> : undefined}>
          {thread.locked
            ? t("reply.locked")
            : !isMember(c)
              ? t("reply.joinFirst")
              : timedOut
                ? t("reply.timedOut")
                : thread.state === community.PostState.HIDDEN
                  ? t("reply.hiddenThread")
                  : t("reply.notAllowed")}
        </Notice>
      )}
      <StampSheet stampId={stampId} onClose={() => setStampId(undefined)} />
    </>
  );
}

function FollowButton({ thread }: { thread: community.Thread }) {
  const { t } = useTranslation(NS);
  const queryClient = useQueryClient();
  const following = thread.viewer?.following ?? false;
  const follow = useMutation({
    mutationFn: (next: boolean) =>
      boardClient().setThreadFollow({ threadId: thread.id, following: next }),
    onSuccess: ({ following: next }) => {
      patchThread(queryClient, thread.id, (each) =>
        each.viewer ? { ...each, viewer: { ...each.viewer, following: next } } : each,
      );
      void queryClient.invalidateQueries({ queryKey: keys.followed });
      toast(next ? t("follow.on") : t("follow.off"));
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: NS })),
  });
  return (
    <Button
      variant={following ? "secondary" : "ghost"}
      size="sm"
      aria-pressed={following}
      loading={follow.isPending}
      icon={following ? <Bell className="size-4" /> : <BellOff className="size-4" />}
      onClick={() => follow.mutate(!following)}
    >
      {following ? t("follow.following") : t("follow.action")}
    </Button>
  );
}

/** The menu of a thread: the author's and the staff's actions, then share and report. */
function ThreadActions({
  community: c,
  thread,
}: {
  community: community.Community;
  thread: community.Thread;
}) {
  const { t } = useTranslation(NS);
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const viewer = thread.viewer;
  const [editing, setEditing] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [closing, setClosing] = useState(false);
  const [revealing, setRevealing] = useState(false);
  const [history, setHistory] = useState(false);
  const share = useShareMenu({
    path: threadHref(c.handle, thread.shortId || thread.id),
    title: thread.title,
    subject: { type: common.SubjectType.COMMUNITY_THREAD, id: thread.id },
    report: !viewer?.mine,
    share: c.visibility !== community.CommunityVisibility.HIDDEN,
  });

  const store = (next: community.Thread | undefined) => {
    if (next) patchThread(queryClient, thread.id, () => next);
    void queryClient.invalidateQueries({ queryKey: keys.threadLists(c.id) });
    void queryClient.invalidateQueries({ queryKey: keys.announcements(c.id) });
  };
  const onError = (error: unknown) => toast.error(errorMessage(error, { namespace: NS }));
  const pin = useMutation({
    mutationFn: (pinned: boolean) => boardClient().pinThread({ threadId: thread.id, pinned }),
    onSuccess: ({ thread: next }) => {
      store(next);
      toast.success(next?.pinned ? t("thread.pinDone") : t("thread.unpinDone"));
    },
    onError,
  });
  const lock = useMutation({
    mutationFn: (locked: boolean) => boardClient().lockThread({ threadId: thread.id, locked }),
    onSuccess: ({ thread: next }) => {
      store(next);
      toast.success(next?.locked ? t("thread.lockDone") : t("thread.unlockDone"));
    },
    onError,
  });
  const announce = useMutation({
    mutationFn: (announcement: boolean) =>
      boardClient().setAnnouncement({ threadId: thread.id, announcement }),
    onSuccess: ({ thread: next }) => {
      store(next);
      toast.success(next?.announcement ? t("thread.announceDone") : t("thread.unannounceDone"));
    },
    onError,
  });
  const closePoll = useMutation({
    mutationFn: () => boardClient().closePoll({ threadId: thread.id }),
    onSuccess: ({ poll }) => {
      setClosing(false);
      if (poll) patchThread(queryClient, thread.id, (each) => ({ ...each, poll }));
    },
    onError,
  });
  const remove = useMutation({
    mutationFn: (reason: string) => boardClient().deleteThread({ threadId: thread.id, reason }),
    onSuccess: () => {
      setDeleting(false);
      void queryClient.invalidateQueries({ queryKey: keys.threadLists(c.id) });
      void queryClient.invalidateQueries({ queryKey: keys.announcements(c.id) });
      queryClient.removeQueries({ queryKey: keys.thread(thread.shortId) });
      queryClient.removeQueries({ queryKey: keys.thread(thread.id) });
      toast(t("thread.deleted"));
      void navigate({ to: "/g/$handle", params: { handle: c.handle } });
    },
    onError,
  });

  const actions: MenuAction[] = [];
  if (viewer?.canEdit)
    actions.push({ label: t("edit"), icon: <Pencil />, onSelect: () => setEditing(true) });
  if (viewer?.canModerate) {
    actions.push(
      {
        label: thread.pinned ? t("thread.unpin") : t("thread.pin"),
        icon: <Pin />,
        onSelect: () => pin.mutate(!thread.pinned),
      },
      {
        label: thread.locked ? t("thread.unlock") : t("thread.lock"),
        icon: thread.locked ? <Unlock /> : <Lock />,
        onSelect: () => lock.mutate(!thread.locked),
      },
    );
  }
  if (can(c, P.ANNOUNCE))
    actions.push({
      label: thread.announcement ? t("thread.unannounce") : t("thread.announce"),
      icon: <Megaphone />,
      onSelect: () => announce.mutate(!thread.announcement),
    });
  if (thread.poll && !thread.poll.closed && (viewer?.mine || viewer?.canModerate))
    actions.push({
      label: t("poll.close"),
      icon: <SquareCheckBig />,
      onSelect: () => setClosing(true),
    });
  if (can(c, P.MODERATE_CONTENT))
    actions.push({
      label: t("history.action"),
      icon: <History />,
      onSelect: () => setHistory(true),
    });
  if (thread.pseudonym && can(c, P.REVEAL_AUTHOR))
    actions.push({ label: t("reveal.action"), icon: <Eye />, onSelect: () => setRevealing(true) });
  actions.push(...share.actions);
  if (viewer?.canDelete)
    actions.push({
      label: t("delete"),
      icon: <Trash2 />,
      danger: true,
      onSelect: () => setDeleting(true),
    });

  return (
    <>
      <Menu
        trigger={
          <Button variant="ghost" size="icon-sm" aria-label={t("thread.more")}>
            <MoreHorizontal className="size-5" />
          </Button>
        }
        actions={actions}
      />
      {share.sheets}
      {editing && <EditThreadDialog thread={thread} onClose={() => setEditing(false)} />}
      {viewer?.mine ? (
        <ConfirmDialog
          open={deleting}
          onOpenChange={setDeleting}
          title={t("thread.deleteTitle")}
          description={t("thread.deleteBody")}
          confirmLabel={t("delete")}
          danger
          loading={remove.isPending}
          onConfirm={() => remove.mutate("")}
        />
      ) : (
        <ReasonDialog
          open={deleting}
          onOpenChange={setDeleting}
          title={t("thread.deleteTitle")}
          description={t("thread.deleteOtherBody")}
          confirmLabel={t("delete")}
          required
          loading={remove.isPending}
          onConfirm={(reason) => remove.mutate(reason)}
        />
      )}
      <ConfirmDialog
        open={closing}
        onOpenChange={setClosing}
        title={t("poll.closeTitle")}
        description={t("poll.closeBody")}
        confirmLabel={t("poll.close")}
        loading={closePoll.isPending}
        onConfirm={() => closePoll.mutate()}
      />
      <RevealAuthorDialog
        post={revealing ? { case: "threadId", value: thread.id } : undefined}
        onClose={() => setRevealing(false)}
      />
      <ContentHistoryDialog
        content={history ? { case: "threadId", value: thread.id } : undefined}
        onClose={() => setHistory(false)}
      />
    </>
  );
}

function EditThreadDialog({ thread, onClose }: { thread: community.Thread; onClose(): void }) {
  const { t } = useTranslation(NS);
  const queryClient = useQueryClient();
  const [title, setTitle] = useState(thread.title);
  const [body, setBody] = useState(thread.body);
  const [tags, setTags] = useState(thread.tags.map((tag) => `#${tag}`).join(" "));
  const save = useMutation({
    mutationFn: () =>
      boardClient().updateThread({
        threadId: thread.id,
        title: title.trim(),
        body: body.trim(),
        tags: { values: parseTags(tags, 5) },
      }),
    onSuccess: ({ thread: next }) => {
      if (next) patchThread(queryClient, thread.id, () => next);
      toast.success(t("saved"));
      onClose();
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: NS })),
  });
  return (
    <Dialog
      open
      onOpenChange={(next) => !next && onClose()}
      title={t("thread.editTitle")}
      description={t("thread.editHint")}
      footer={
        <Button loading={save.isPending} disabled={!title.trim()} onClick={() => save.mutate()}>
          {t("save")}
        </Button>
      }
    >
      <div className="flex flex-col gap-4">
        <TextField
          label={t("compose.title")}
          value={title}
          maxLength={100}
          onChange={(event) => setTitle(event.target.value)}
        />
        <PostTextArea
          label={t("compose.body")}
          value={body}
          onChange={setBody}
          maxLength={5000}
          rows={6}
        />
        <TextField
          label={t("compose.tags")}
          hint={t("compose.tagsHint")}
          value={tags}
          onChange={(event) => setTags(event.target.value)}
        />
      </div>
    </Dialog>
  );
}

function ReplyItem({
  community: c,
  thread,
  reply,
  onReplyTo,
  onOpenStamp,
}: {
  community: community.Community;
  thread: community.Thread;
  reply: community.Reply;
  onReplyTo?: () => void;
  onOpenStamp(stampId: string): void;
}) {
  const { t, i18n } = useTranslation(NS);
  const queryClient = useQueryClient();
  const [editing, setEditing] = useState<string | null>(null);
  const [deleting, setDeleting] = useState(false);
  const [revealing, setRevealing] = useState(false);
  const [history, setHistory] = useState(false);
  const viewer = reply.viewer;
  const subject = { type: common.SubjectType.COMMUNITY_REPLY, id: reply.id };
  const share = useShareMenu({
    path: threadHref(c.handle, thread.shortId || thread.id),
    title: reply.body.slice(0, 60) || thread.title,
    subject,
    share: false,
    report: !viewer?.mine,
  });
  const onError = (error: unknown) => toast.error(errorMessage(error, { namespace: NS }));
  const save = useMutation({
    mutationFn: (body: string) => boardClient().updateReply({ replyId: reply.id, body }),
    onSuccess: ({ reply: next }) => {
      if (next) upsertReply(queryClient, next);
      setEditing(null);
    },
    onError,
  });
  const remove = useMutation({
    mutationFn: (reason: string) => boardClient().deleteReply({ replyId: reply.id, reason }),
    onSuccess: () => {
      setDeleting(false);
      patchReplies(queryClient, thread.id, (replies) =>
        replies.filter((each) => each.id !== reply.id),
      );
      patchThread(queryClient, thread.id, (each) => ({
        ...each,
        replyCount: Math.max(0, each.replyCount - 1),
      }));
      toast(t("reply.deleted"));
    },
    onError,
  });

  const actions: MenuAction[] = [];
  if (viewer?.canEdit)
    actions.push({ label: t("edit"), icon: <Pencil />, onSelect: () => setEditing(reply.body) });
  if (can(c, P.MODERATE_CONTENT))
    actions.push({
      label: t("history.action"),
      icon: <History />,
      onSelect: () => setHistory(true),
    });
  if (reply.pseudonym && can(c, P.REVEAL_AUTHOR))
    actions.push({ label: t("reveal.action"), icon: <Eye />, onSelect: () => setRevealing(true) });
  actions.push(...share.actions);
  if (viewer?.canDelete)
    actions.push({
      label: t("delete"),
      icon: <Trash2 />,
      danger: true,
      onSelect: () => setDeleting(true),
    });

  const quoted = reply.replyTo;
  return (
    <article
      className={cn(
        "rounded-card bg-surface p-4",
        reply.state === community.PostState.HIDDEN && "border-2 border-dashed border-coral",
      )}
    >
      <div className="flex items-start gap-2">
        <div className="min-w-0 flex-1">
          <PostAuthor
            size="sm"
            author={reply.author}
            pseudonym={reply.pseudonym}
            mine={viewer?.mine}
            createdAtMs={Number(reply.createdAt?.seconds ?? 0n) * 1000}
            edited={Boolean(reply.editedAt)}
            badges={
              reply.byThreadAuthor ? (
                <Chip className="bg-lime text-[#17122b]">{t("reply.byThreadAuthor")}</Chip>
              ) : undefined
            }
          />
        </div>
        <span className="pt-1 text-xs font-bold text-ink-faint">#{reply.number}</span>
        {actions.length > 0 && (
          <Menu
            trigger={
              <Button
                variant="ghost"
                size="icon-sm"
                aria-label={t("reply.more", { number: reply.number })}
              >
                <MoreHorizontal className="size-5" />
              </Button>
            }
            actions={actions}
          />
        )}
      </div>
      {reply.state === community.PostState.HIDDEN && (
        <Notice tone="warn" className="mt-2">
          {reply.hiddenReason === community.HiddenReason.MUTED_WORD
            ? t("hidden.mutedWord")
            : t("hidden.reports")}
        </Notice>
      )}
      {quoted && (
        <div className="mt-2 flex items-start gap-1.5 rounded-xl bg-surface-2 px-3 py-2 text-sm text-ink-soft">
          <CornerDownRight className="mt-0.5 size-4 shrink-0" />
          <p className="min-w-0 flex-1 break-words">
            {quoted.unavailable ? (
              t("reply.quoteGone")
            ) : (
              <>
                <span className="font-bold">
                  {quoted.pseudonym
                    ? pseudonymName(quoted.pseudonym, i18n.language)
                    : quoted.author?.displayName || quoted.author?.handle}
                </span>{" "}
                <EmojiText text={quoted.excerpt} />
              </>
            )}
          </p>
        </div>
      )}
      {editing !== null ? (
        <div className="mt-2 flex flex-col gap-2">
          <PostTextArea
            label={t("reply.editLabel")}
            value={editing}
            onChange={setEditing}
            maxLength={2000}
            rows={3}
          />
          <div className="flex justify-end gap-2">
            <Button variant="ghost" size="sm" onClick={() => setEditing(null)}>
              {t("cancel")}
            </Button>
            <Button size="sm" loading={save.isPending} onClick={() => save.mutate(editing.trim())}>
              {t("save")}
            </Button>
          </div>
        </div>
      ) : (
        <PostBody body={reply.body} mentions={reply.mentions} className="mt-2" />
      )}
      {reply.images.length > 0 && (
        <div className="mt-2">
          <PostImages images={reply.images} />
        </div>
      )}
      {reply.stamp && (
        <button
          type="button"
          className="mt-2 block"
          onClick={() => reply.stamp && onOpenStamp(reply.stamp.id)}
          aria-label={reply.stamp.name || t("compose.stamp")}
        >
          <StampImage stamp={reply.stamp} size={104} />
        </button>
      )}
      <div className="mt-2 flex flex-wrap items-center gap-2">
        <ReactionBar subject={subject} className="min-w-0 flex-1" />
        {onReplyTo && (
          <Button
            variant="ghost"
            size="sm"
            icon={<ReplyIcon className="size-4" />}
            onClick={onReplyTo}
            aria-label={t("reply.replyTo", { number: reply.number })}
          >
            {t("reply.action")}
          </Button>
        )}
      </div>
      {share.sheets}
      {viewer?.mine ? (
        <ConfirmDialog
          open={deleting}
          onOpenChange={setDeleting}
          title={t("reply.deleteTitle")}
          confirmLabel={t("delete")}
          danger
          loading={remove.isPending}
          onConfirm={() => remove.mutate("")}
        />
      ) : (
        <ReasonDialog
          open={deleting}
          onOpenChange={setDeleting}
          title={t("reply.deleteTitle")}
          description={t("thread.deleteOtherBody")}
          confirmLabel={t("delete")}
          required
          loading={remove.isPending}
          onConfirm={(reason) => remove.mutate(reason)}
        />
      )}
      <RevealAuthorDialog
        post={revealing ? { case: "replyId", value: reply.id } : undefined}
        onClose={() => setRevealing(false)}
      />
      <ContentHistoryDialog
        content={history ? { case: "replyId", value: reply.id } : undefined}
        onClose={() => setHistory(false)}
      />
    </article>
  );
}
