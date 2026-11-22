import { type QueryClient, useInfiniteQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { common, create, engagement, toMillis } from "@tokuly/api";
import { Ban, ChevronDown, ChevronUp, Clock, EyeOff, Heart, MoreVertical, Pencil, Pin, PinOff, ThumbsDown, ThumbsUp, Trash2 } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { Dialog } from "#/components/ui/dialog";
import { Menu, type MenuAction } from "#/components/ui/menu";
import { Spinner } from "#/components/ui/spinner";
import { StampImage, StampSheet } from "#/features/stamps";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { compactAge } from "#/lib/format";
import { commentClient, commentKeys, moderationClient, reactionClient, type SubjectKey, updateReplies, updateThread } from "./api";
import { AuthorAvatar, AuthorLink, authorName, sameAuthor, VerifiedMark } from "./author";
import { withoutComment, withoutReply, withPatched, withPatchedReply, withPinned, withUpdated, withUpdatedReply } from "./cache";
import { formatClock } from "./clock";
import { Composer } from "./composer";
import { tokenize } from "./text";
import { useThread } from "./thread-context";

type Comment = engagement.Comment;
const { CommentStatus, ReactionKind } = engagement;

/** Changes one cached comment, wherever it is listed (the thread, or its parent's replies). */
function patchComment(queryClient: QueryClient, subject: SubjectKey, comment: Comment, patch: (comment: Comment) => Comment): void {
  if (comment.parentId) updateReplies(queryClient, comment.parentId, (data) => withPatchedReply(data, comment.id, patch));
  else updateThread(queryClient, subject, (data) => withPatched(data, comment.id, patch));
}

/** Stores the server's version of a comment after a change. */
function putComment(queryClient: QueryClient, subject: SubjectKey, comment: Comment): void {
  if (comment.parentId) updateReplies(queryClient, comment.parentId, (data) => withUpdatedReply(data, comment));
  else updateThread(queryClient, subject, (data) => withUpdated(data, comment));
}

/** Takes a comment out of the lists; a public one also comes off the counts. */
function dropComment(queryClient: QueryClient, subject: SubjectKey, comment: Comment, removedCount: number): void {
  const counted = comment.status === CommentStatus.PUBLISHED;
  if (comment.parentId) {
    updateReplies(queryClient, comment.parentId, (data) => withoutReply(data, comment.id));
    if (counted) updateThread(queryClient, subject, (data) => withPatched(data, comment.parentId, (parent) => ({ ...parent, replyCount: Math.max(0, parent.replyCount - 1) })));
  } else {
    updateThread(queryClient, subject, (data) => withoutComment(data, comment.id, counted ? removedCount : 0));
  }
}

/** When a comment was written: an age for recent ones, the date after a month. */
function writtenAt(comment: Comment, language: string): string {
  const at = toMillis(comment.createdAt);
  return Date.now() - at < 30 * 86_400_000 ? compactAge(at) : new Date(at).toLocaleDateString(language, { year: "numeric", month: "short", day: "numeric" });
}

export interface CommentItemProps {
  comment: Comment;
  /** Draws attention to this comment and scrolls it into view (a `?comment=` deep link). */
  focused?: boolean;
  /** A reply to show (and highlight) under this comment even before its page of replies is loaded. */
  focusReply?: Comment;
  /** For replies: answer this reply (the composer lives with the thread's root). */
  onReply?(comment: Comment): void;
}

/**
 * One comment with everything that can be done to it: like / dislike, reply,
 * edit and delete for its author, pin, heart, hide and block for the
 * subject's owner — and, for a top-level comment, its replies on demand.
 */
export function CommentItem({ comment, focused, focusReply, onReply }: CommentItemProps) {
  const { t, i18n } = useTranslation("comments");
  const thread = useThread();
  const queryClient = useQueryClient();
  const root = useRef<HTMLElement>(null);
  const isReply = comment.parentId !== "";
  const viewer = comment.viewer;
  const published = comment.status === CommentStatus.PUBLISHED;

  const [editing, setEditing] = useState(false);
  const [confirming, setConfirming] = useState<"delete" | "block" | null>(null);
  const [repliesOpen, setRepliesOpen] = useState(Boolean(focusReply));
  const [replyingTo, setReplyingTo] = useState<Comment | null>(null);
  const [openStamp, setOpenStamp] = useState<string | undefined>();
  const [imageOpen, setImageOpen] = useState(false);

  useEffect(() => {
    if (focused) root.current?.scrollIntoView({ block: "center", behavior: "smooth" });
  }, [focused]);

  const onError = (error: unknown) => toast.error(errorMessage(error));
  const liked = viewer?.reaction?.kind === ReactionKind.LIKE;
  const disliked = viewer?.reaction?.kind === ReactionKind.DISLIKE;

  const react = useMutation({
    mutationFn: async (kind: engagement.ReactionKind | null) => {
      const subject = { type: common.SubjectType.COMMENT, id: comment.id };
      const response = kind === null ? await reactionClient().removeReaction({ subject }) : await reactionClient().setReaction({ subject, reaction: { kind } });
      return response.reactions;
    },
    // The thumb answers the tap at once; the server's counts follow.
    onMutate: (kind) => {
      patchComment(queryClient, thread.subject, comment, (current) => ({
        ...current,
        likeCount: current.likeCount + (kind === ReactionKind.LIKE ? 1n : 0n) - (liked ? 1n : 0n),
        dislikeCount: current.dislikeCount + (kind === ReactionKind.DISLIKE ? 1n : 0n) - (disliked ? 1n : 0n),
        viewer: current.viewer && { ...current.viewer, reaction: kind === null ? undefined : create(engagement.ReactionSchema, { kind }) },
      }));
      return { previous: comment };
    },
    onSuccess: (reactions) => {
      if (reactions) patchComment(queryClient, thread.subject, comment, (current) => ({ ...current, likeCount: reactions.likeCount, dislikeCount: reactions.dislikeCount }));
    },
    onError: (error, _kind, context) => {
      if (context) patchComment(queryClient, thread.subject, comment, () => context.previous);
      onError(error);
    },
  });

  const remove = useMutation({
    mutationFn: () => commentClient().deleteComment({ commentId: comment.id }),
    onSuccess: ({ removedCount }) => {
      setConfirming(null);
      dropComment(queryClient, thread.subject, comment, removedCount);
      void queryClient.invalidateQueries({ queryKey: commentKeys.mine });
      toast.success(t("toast.deleted"));
    },
    onError: (error) => {
      setConfirming(null);
      onError(error);
    },
  });
  const pin = useMutation({
    mutationFn: async (pinned: boolean) => {
      if (pinned) await commentClient().pinComment({ commentId: comment.id });
      else await commentClient().unpinComment({ commentId: comment.id });
    },
    onSuccess: (_response, pinned) => {
      updateThread(queryClient, thread.subject, (data) => withPinned(data, comment.id, pinned));
      toast.success(pinned ? t("toast.pinned") : t("toast.unpinned"));
    },
    onError,
  });
  const heart = useMutation({
    mutationFn: (hearted: boolean) => commentClient().setCommentHeart({ commentId: comment.id, hearted }),
    onMutate: (hearted) => patchComment(queryClient, thread.subject, comment, (current) => ({ ...current, hearted })),
    onError: (error, hearted) => {
      patchComment(queryClient, thread.subject, comment, (current) => ({ ...current, hearted: !hearted }));
      onError(error);
    },
  });
  const hide = useMutation({
    mutationFn: () => moderationClient().hideComment({ commentId: comment.id }),
    onSuccess: () => {
      dropComment(queryClient, thread.subject, comment, 1 + comment.replyCount);
      void queryClient.invalidateQueries({ queryKey: commentKeys.received });
      toast.success(t("toast.hidden"));
    },
    onError,
  });
  const block = useMutation({
    mutationFn: () => moderationClient().blockCommenter({ target: { case: "commentId", value: comment.id } }),
    onSuccess: () => {
      setConfirming(null);
      // Everything they wrote here is hidden now: read the thread again.
      void queryClient.invalidateQueries({ queryKey: commentKeys.subject(thread.subject) });
      void queryClient.invalidateQueries({ queryKey: commentKeys.blocked });
      toast.success(t("toast.blocked", { name: authorName(comment.author) }));
    },
    onError: (error) => {
      setConfirming(null);
      onError(error);
    },
  });

  const actions: MenuAction[] = [];
  if (viewer?.canEdit) actions.push({ label: t("actions.edit"), icon: <Pencil />, onSelect: () => setEditing(true) });
  if (viewer?.canModerate && !isReply && published) {
    actions.push(comment.pinned ? { label: t("actions.unpin"), icon: <PinOff />, onSelect: () => pin.mutate(false) } : { label: t("actions.pin"), icon: <Pin />, onSelect: () => pin.mutate(true) });
  }
  if (viewer?.canModerate && !viewer.isAuthor) {
    if (published) actions.push({ label: t("actions.hide"), icon: <EyeOff />, onSelect: () => hide.mutate() });
    actions.push({ label: t("actions.block"), icon: <Ban />, onSelect: () => setConfirming("block") });
  }
  if (viewer?.canDelete) actions.push({ label: t("actions.delete"), icon: <Trash2 />, danger: true, onSelect: () => setConfirming("delete") });

  const isCreator = sameAuthor(comment.author, thread.subjectAuthor);
  const name = authorName(comment.author);
  const startReply = () => {
    if (onReply) onReply(comment);
    else {
      setRepliesOpen(true);
      setReplyingTo(comment);
    }
  };

  return (
    <article
      ref={root}
      data-testid="comment"
      data-comment-id={comment.id}
      aria-label={t("item.label", { name })}
      className={cn("flex gap-3 rounded-2xl", focused && "bg-brand-soft/60 p-3 ring-2 ring-brand", !published && "opacity-80")}
    >
      <AuthorLink author={comment.author} className="shrink-0 self-start">
        <AuthorAvatar author={comment.author} size={isReply ? "xs" : "sm"} />
      </AuthorLink>
      <div className="min-w-0 flex-1">
        {comment.pinned && (
          <p className="mb-0.5 flex items-center gap-1 text-xs font-black text-ink-soft">
            <Pin className="size-3.5" />
            {t("item.pinned")}
          </p>
        )}
        <div className="flex flex-wrap items-center gap-x-2 gap-y-0.5 text-sm">
          <AuthorLink author={comment.author} className="max-w-full truncate font-black">
            {name || t("item.unknown")}
          </AuthorLink>
          <VerifiedMark author={comment.author} label={t("item.verified")} />
          {isCreator && <span className="inline-flex h-5 items-center rounded-full bg-ink px-2 text-[11px] font-black text-bg">{t("item.creator")}</span>}
          <span className="text-xs text-ink-faint">
            {writtenAt(comment, i18n.language)}
            {comment.edited && ` · ${t("item.edited")}`}
          </span>
          {comment.status === CommentStatus.HELD && <StatusChip tone="held">{t("status.held")}</StatusChip>}
          {comment.status === CommentStatus.HIDDEN && <StatusChip tone="hidden">{t("status.hidden")}</StatusChip>}
        </div>

        {editing ? (
          <EditForm comment={comment} onDone={() => setEditing(false)} />
        ) : (
          <>
            {comment.timestampSeconds !== undefined && (
              <button
                type="button"
                className="mt-1 inline-flex h-7 items-center gap-1 rounded-full bg-brand-soft px-2.5 text-xs font-black text-brand-strong disabled:cursor-default"
                disabled={!thread.video}
                onClick={() => thread.video?.seekTo(comment.timestampSeconds ?? 0)}
                aria-label={t("item.jumpTo", { time: formatClock(comment.timestampSeconds) })}
              >
                <Clock className="size-3.5" />
                {formatClock(comment.timestampSeconds)}
              </button>
            )}
            {comment.text && <CommentText comment={comment} />}
            {comment.stamp &&
              (comment.stamp.imageUrl || comment.stamp.animationUrl ? (
                <button type="button" className="mt-1.5 block rounded-2xl" aria-label={t("item.openStamp", { name: comment.stamp.name })} onClick={() => setOpenStamp(comment.stamp?.id)}>
                  <StampImage stamp={comment.stamp} size={isReply ? 88 : 112} />
                </button>
              ) : (
                <p className="mt-1.5 inline-block rounded-xl border-2 border-dashed border-line px-3 py-2 text-xs font-bold text-ink-faint">{t("item.stampGone")}</p>
              ))}
            {comment.image?.url && (
              <button type="button" className="mt-1.5 block overflow-hidden rounded-2xl border-2 border-line" aria-label={t("item.openImage")} onClick={() => setImageOpen(true)}>
                <img
                  src={comment.image.variants.find((variant) => variant.size >= 480)?.url ?? comment.image.url}
                  alt=""
                  loading="lazy"
                  className="max-h-60 max-w-full object-cover"
                  style={comment.image.dominantColor ? { background: comment.image.dominantColor } : undefined}
                />
              </button>
            )}
            {comment.status === CommentStatus.HELD && viewer?.isAuthor && <p className="mt-1 text-xs leading-relaxed text-ink-soft">{t("status.heldHint")}</p>}

            <div className="-ml-2 mt-0.5 flex flex-wrap items-center gap-0.5">
              <button
                type="button"
                aria-pressed={liked}
                aria-label={t("actions.like")}
                disabled={!published || !viewer}
                className={cn("inline-flex h-8 items-center gap-1 rounded-full px-2 text-xs font-black hover:bg-surface-2 disabled:opacity-50", liked ? "text-brand" : "text-ink-soft")}
                onClick={() => react.mutate(liked ? null : ReactionKind.LIKE)}
              >
                <ThumbsUp className={cn("size-4", liked && "fill-current")} />
                {comment.likeCount > 0n && <span className="tabular-nums">{Number(comment.likeCount).toLocaleString(i18n.language)}</span>}
              </button>
              <button
                type="button"
                aria-pressed={disliked}
                aria-label={t("actions.dislike")}
                disabled={!published || !viewer}
                className={cn("grid size-8 place-items-center rounded-full hover:bg-surface-2 disabled:opacity-50", disliked ? "text-ink" : "text-ink-soft")}
                onClick={() => react.mutate(disliked ? null : ReactionKind.DISLIKE)}
              >
                <ThumbsDown className={cn("size-4", disliked && "fill-current")} />
              </button>
              {viewer?.canModerate && published ? (
                <button
                  type="button"
                  aria-pressed={comment.hearted}
                  aria-label={t("actions.heart")}
                  className={cn("grid size-8 place-items-center rounded-full hover:bg-surface-2", comment.hearted ? "text-pink" : "text-ink-soft")}
                  onClick={() => heart.mutate(!comment.hearted)}
                >
                  <Heart className={cn("size-4", comment.hearted && "fill-current")} />
                </button>
              ) : (
                comment.hearted && (
                  <span className="grid size-8 place-items-center text-pink" role="img" aria-label={t("item.hearted")} title={t("item.hearted")}>
                    <Heart className="size-4 fill-current" />
                  </span>
                )
              )}
              {published && viewer && (
                <button type="button" className="h-8 rounded-full px-3 text-xs font-black text-ink-soft hover:bg-surface-2" onClick={startReply}>
                  {t("actions.reply")}
                </button>
              )}
              {actions.length > 0 && (
                <Menu
                  trigger={
                    <button type="button" aria-label={t("actions.more")} className="grid size-8 place-items-center rounded-full text-ink-soft hover:bg-surface-2">
                      <MoreVertical className="size-4" />
                    </button>
                  }
                  actions={actions}
                />
              )}
            </div>
          </>
        )}

        {!isReply && (comment.replyCount > 0 || replyingTo || focusReply) && (
          <div className="mt-1">
            {(comment.replyCount > 0 || focusReply) && (
              <button
                type="button"
                aria-expanded={repliesOpen}
                className="inline-flex h-8 items-center gap-1 rounded-full px-2.5 text-sm font-black text-brand hover:bg-brand-soft"
                onClick={() => setRepliesOpen((open) => !open)}
              >
                {repliesOpen ? <ChevronUp className="size-4" /> : <ChevronDown className="size-4" />}
                {t("replies.toggle", { count: Math.max(comment.replyCount, focusReply ? 1 : 0) })}
              </button>
            )}
            {repliesOpen && <Replies parent={comment} focusReply={focusReply} onReply={setReplyingTo} />}
            {replyingTo && (
              <div className="mt-3">
                <Composer key={replyingTo.id} replyTo={replyingTo} autoFocus onCancel={() => setReplyingTo(null)} onPosted={() => setReplyingTo(null)} />
              </div>
            )}
          </div>
        )}
      </div>

      <ConfirmDialog
        open={confirming === "delete"}
        onOpenChange={(open) => !open && setConfirming(null)}
        title={t("confirm.deleteTitle")}
        description={!isReply && comment.replyCount > 0 ? t("confirm.deleteWithReplies", { count: comment.replyCount }) : t("confirm.deleteBody")}
        confirmLabel={t("actions.delete")}
        danger
        loading={remove.isPending}
        onConfirm={() => remove.mutate()}
      />
      <ConfirmDialog
        open={confirming === "block"}
        onOpenChange={(open) => !open && setConfirming(null)}
        title={t("confirm.blockTitle", { name })}
        description={t("confirm.blockBody")}
        confirmLabel={t("actions.block")}
        danger
        loading={block.isPending}
        onConfirm={() => block.mutate()}
      />
      <StampSheet stampId={openStamp} onClose={() => setOpenStamp(undefined)} />
      {comment.image?.url && (
        <Dialog open={imageOpen} onOpenChange={setImageOpen} title={t("item.imageTitle", { name })} className="sm:w-[min(48rem,calc(100vw-2rem))]">
          <img src={comment.image.url} alt={t("item.imageTitle", { name })} className="mx-auto max-h-[70dvh] max-w-full rounded-2xl object-contain" />
        </Dialog>
      )}
    </article>
  );
}

function StatusChip({ tone, children }: { tone: "held" | "hidden"; children: string }) {
  return <span className={cn("inline-flex h-5 items-center rounded-full px-2 text-[11px] font-black", tone === "held" ? "bg-sun text-[#17122b]" : "bg-surface-3 text-ink-soft")}>{children}</span>;
}

/** Above this the text is folded behind "read more". */
const FOLD_CHARACTERS = 360;
const FOLD_LINES = 6;

/** The comment's words, with mentions, links and — on videos — clock times made tappable. */
function CommentText({ comment }: { comment: Comment }) {
  const { t } = useTranslation("comments");
  const thread = useThread();
  const [expanded, setExpanded] = useState(false);
  const handles = new Set(comment.mentions.map((user) => user.handle.toLowerCase()));
  const tokens = tokenize(comment.text, handles, Boolean(thread.video));
  const long = comment.text.length > FOLD_CHARACTERS || comment.text.split("\n").length > FOLD_LINES;

  return (
    <div className="mt-0.5">
      <p data-testid="comment-text" className={cn("text-[15px] leading-relaxed break-words whitespace-pre-wrap", long && !expanded && "line-clamp-6")}>
        {tokens.map((token, index) => {
          if (token.kind === "mention") {
            return (
              <Link key={index} to="/u/$handle" params={{ handle: token.handle }} className="font-bold text-brand hover:underline">
                {token.text}
              </Link>
            );
          }
          if (token.kind === "link") {
            return (
              <a key={index} href={token.url} target="_blank" rel="noopener noreferrer nofollow ugc" className="font-bold text-brand underline-offset-2 hover:underline">
                {token.text}
              </a>
            );
          }
          if (token.kind === "time") {
            return (
              <button key={index} type="button" className="font-bold text-brand hover:underline" onClick={() => thread.video?.seekTo(token.seconds)} aria-label={t("item.jumpTo", { time: token.text })}>
                {token.text}
              </button>
            );
          }
          return token.text;
        })}
      </p>
      {long && (
        <button type="button" className="mt-0.5 text-sm font-black text-ink-soft hover:text-ink" onClick={() => setExpanded((value) => !value)}>
          {expanded ? t("item.showLess") : t("item.showMore")}
        </button>
      )}
    </div>
  );
}

/** Changing the words of one's own comment, in place. */
function EditForm({ comment, onDone }: { comment: Comment; onDone(): void }) {
  const { t } = useTranslation("comments");
  const thread = useThread();
  const queryClient = useQueryClient();
  const [text, setText] = useState(comment.text);
  const hasAttachment = Boolean(comment.stamp || comment.image);

  const save = useMutation({
    mutationFn: () => commentClient().updateComment({ commentId: comment.id, text: text.trim() }),
    onSuccess: ({ comment: updated }) => {
      if (updated) {
        putComment(queryClient, thread.subject, updated);
        // The new words may have tripped the owner's filters.
        if (comment.status === CommentStatus.PUBLISHED && updated.status === CommentStatus.HELD) toast.message(t("composer.held"));
      }
      void queryClient.invalidateQueries({ queryKey: commentKeys.mine });
      onDone();
    },
    onError: (error) => toast.error(errorMessage(error)),
  });
  const unchanged = text.trim() === comment.text.trim();
  const canSave = !unchanged && text.length <= 1000 && (text.trim().length > 0 || hasAttachment);

  return (
    <form
      className="mt-1.5"
      onSubmit={(event) => {
        event.preventDefault();
        if (canSave) save.mutate();
      }}
    >
      <textarea
        // biome-ignore lint/a11y/noAutofocus: the person just chose "edit".
        autoFocus
        value={text}
        maxLength={1200}
        aria-label={t("actions.edit")}
        className="field-sizing-content block max-h-60 min-h-20 w-full resize-none rounded-control border-2 border-line bg-surface px-4 py-2.5 text-[15px] leading-relaxed outline-none focus:border-brand"
        onChange={(event) => setText(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Escape") onDone();
          else if (event.key === "Enter" && (event.metaKey || event.ctrlKey) && canSave) save.mutate();
        }}
      />
      <div className="mt-2 flex justify-end gap-2">
        <Button variant="ghost" size="sm" onClick={onDone}>
          {t("composer.cancel")}
        </Button>
        <Button type="submit" size="sm" disabled={!canSave} loading={save.isPending}>
          {t("composer.save")}
        </Button>
      </div>
    </form>
  );
}

/** The replies under a top-level comment, oldest first, a page at a time. */
function Replies({ parent, focusReply, onReply }: { parent: Comment; focusReply?: Comment; onReply(comment: Comment): void }) {
  const { t } = useTranslation("comments");
  const query = useInfiniteQuery({
    queryKey: commentKeys.replies(parent.id),
    queryFn: ({ pageParam }) => commentClient().listReplies({ commentId: parent.id, page: { limit: 20, cursor: pageParam } }),
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
  });
  const loaded = query.data?.pages.flatMap((page) => page.replies) ?? [];
  // The reply a link pointed at is shown even when its page has not been reached yet.
  const replies = focusReply && !loaded.some((reply) => reply.id === focusReply.id) ? [...loaded, focusReply] : loaded;

  return (
    <div className="mt-2 flex flex-col gap-3">
      {query.isPending && (
        <div className="grid h-10 place-items-center text-ink-faint">
          <Spinner />
        </div>
      )}
      {query.isError && <p className="text-sm font-bold text-ink-faint">{t("replies.error")}</p>}
      {replies.map((reply) => (
        <CommentItem key={reply.id} comment={reply} focused={reply.id === focusReply?.id} onReply={onReply} />
      ))}
      {query.hasNextPage && (
        <button
          type="button"
          className="inline-flex h-8 items-center gap-1 self-start rounded-full px-2.5 text-sm font-black text-brand hover:bg-brand-soft disabled:opacity-50"
          disabled={query.isFetchingNextPage}
          onClick={() => void query.fetchNextPage()}
        >
          {query.isFetchingNextPage ? <Spinner className="size-4" /> : <ChevronDown className="size-4" />}
          {t("replies.more")}
        </button>
      )}
    </div>
  );
}
