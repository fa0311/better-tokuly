import { useInfiniteQuery, useQuery, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { type common, engagement } from "@tokuly/api";
import { MessageSquareOff, Settings2 } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { Tabs } from "#/components/ui/tabs";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { useMe } from "#/stores/session";
import { commentClient, commentKeys, isUuid, type SubjectKey, updateThread } from "./api";
import { findComment, withComment } from "./cache";
import { CommentItem } from "./comment-item";
import { Composer } from "./composer";
import { useThreadRealtime } from "./realtime";
import { type CommentVideoBridge, ThreadContext } from "./thread-context";

const { CommentSort } = engagement;
type SortTab = "top" | "new";

export interface CommentThreadProps {
  /**
   * What the comments are on. `id` may be the subject's UUID or its short ID;
   * live updates need the UUID (with a short ID they start once the first
   * comment is loaded, which names it).
   */
  subject: Pick<common.SubjectRef, "type" | "id">;
  /** The subject's own public author (for a video: `{ channel }`): their comments get the creator badge. */
  subjectAuthor?: common.Author;
  /** A comment to bring to the top and highlight: the `?comment=<id>` of a notification link. */
  focusCommentId?: string;
  /**
   * For videos: lets a comment carry the current moment of the player, and
   * makes those moments (and clock times in the text) jump there.
   */
  video?: CommentVideoBridge;
  /** One of the viewer's channels to comment as by default (the subject's own channel, for its owner). */
  defaultChannelId?: string;
  className?: string;
}

/**
 * The comments of any subject — a video, a note, a short, a stream: sorted
 * top or new, one level of replies loaded on demand, a composer with
 * mentions and an optional stamp or image, edit and delete for authors, pin,
 * heart, hide and block for the subject's owner, and new comments arriving
 * live while it is on screen.
 */
export function CommentThread({
  subject,
  subjectAuthor,
  focusCommentId,
  video,
  defaultChannelId,
  className,
}: CommentThreadProps) {
  const { t, i18n } = useTranslation("comments");
  const queryClient = useQueryClient();
  const me = useMe();
  const [sort, setSort] = useState<SortTab>("top");
  const key: SubjectKey = { type: subject.type, id: subject.id };

  const query = useInfiniteQuery({
    queryKey: commentKeys.thread(key, sort === "top" ? CommentSort.TOP : CommentSort.NEWEST),
    queryFn: ({ pageParam }) =>
      commentClient().listComments({
        subject: key,
        sort: sort === "top" ? CommentSort.TOP : CommentSort.NEWEST,
        page: { limit: 20, cursor: pageParam },
      }),
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
  });
  const first = query.data?.pages[0];
  const canModerate = first?.viewerCanModerate ?? false;
  const commentsEnabled = first?.commentsEnabled ?? true;
  const total = Number(first?.totalCount ?? 0n);

  const focus = useQuery({
    queryKey: commentKeys.focused(focusCommentId ?? ""),
    queryFn: () => commentClient().getComment({ commentId: focusCommentId ?? "" }),
    enabled: Boolean(focusCommentId),
    retry: false,
  });
  const focusRoot = focus.data?.parent ?? focus.data?.comment;
  const focusReply = focus.data?.parent ? focus.data.comment : undefined;

  // The comment a link points at joins the list (once), so everything done to it lands in one place.
  const loaded = query.data;
  useEffect(() => {
    if (focusRoot && loaded && !findComment(loaded, focusRoot.id))
      updateThread(queryClient, { type: subject.type, id: subject.id }, (data) =>
        withComment(data, focusRoot, false),
      );
  }, [focusRoot, loaded, queryClient, subject.type, subject.id]);

  const seen = new Set<string>();
  const comments = (query.data?.pages.flatMap((page) => page.comments) ?? []).filter(
    (comment) => !seen.has(comment.id) && seen.add(comment.id),
  );
  // Pinned first, then the comment the link pointed at, then the rest in server order.
  const ordered = focusRoot
    ? [
        ...comments.filter((comment) => comment.pinned),
        ...comments.filter((comment) => !comment.pinned && comment.id === focusRoot.id),
        ...comments.filter((comment) => !comment.pinned && comment.id !== focusRoot.id),
      ]
    : comments;

  const uuid = isUuid(subject.id) ? subject.id : comments[0]?.subject?.id;
  useThreadRealtime(key, uuid, canModerate);

  return (
    <ThreadContext value={{ subject: key, canModerate, subjectAuthor, video, defaultChannelId }}>
      <section
        aria-label={t("title")}
        data-testid="comment-thread"
        className={cn("flex flex-col gap-4", className)}
      >
        <header className="flex flex-wrap items-center gap-x-3 gap-y-2">
          <h2 className="text-lg font-black">
            {t("title")}
            {query.data && (
              <span
                className="ml-2 text-base font-bold text-ink-soft tabular-nums"
                data-testid="comment-count"
              >
                {t("count", { count: total, formatted: total.toLocaleString(i18n.language) })}
              </span>
            )}
          </h2>
          <span className="flex-1" />
          {canModerate && (
            <Link
              to="/settings/comments"
              className="inline-flex h-9 items-center gap-1.5 rounded-full px-3 text-sm font-bold text-ink-soft hover:bg-surface-2 hover:text-ink"
            >
              <Settings2 className="size-4" />
              {t("manage")}
            </Link>
          )}
          <Tabs<SortTab>
            value={sort}
            onChange={setSort}
            items={[
              { value: "top", label: t("sort.top") },
              { value: "new", label: t("sort.new") },
            ]}
          />
        </header>

        {!commentsEnabled && query.data && (
          <p className="flex items-center gap-2 rounded-2xl bg-surface-2 px-4 py-3 text-sm font-bold text-ink-soft">
            <MessageSquareOff className="size-5 shrink-0" />
            {canModerate ? t("disabledOwner") : t("disabled")}
          </p>
        )}
        {commentsEnabled && query.data && me && <Composer />}

        {query.isPending ? (
          <div className="flex flex-col gap-4" aria-busy>
            {[0, 1, 2].map((row) => (
              <div key={row} className="flex gap-3">
                <Skeleton className="size-8 shrink-0 rounded-[0.6rem]" />
                <div className="flex-1">
                  <Skeleton className="h-4 w-32" />
                  <Skeleton className="mt-2 h-4 w-full max-w-md" />
                </div>
              </div>
            ))}
          </div>
        ) : query.isError ? (
          <p
            role="alert"
            className="rounded-2xl bg-surface-2 px-4 py-6 text-center text-sm font-bold text-ink-soft"
          >
            {errorMessage(query.error)}
          </p>
        ) : ordered.length === 0 ? (
          commentsEnabled && (
            <EmptyState emoji="💬" title={t("empty.title")} body={t("empty.body")} />
          )
        ) : (
          <div className="flex flex-col gap-5">
            {ordered.map((comment) => (
              <CommentItem
                key={comment.id}
                comment={comment}
                focused={comment.id === focusRoot?.id && !focusReply}
                focusReply={comment.id === focusRoot?.id ? focusReply : undefined}
              />
            ))}
            <LoadMore query={query} />
          </div>
        )}
      </section>
    </ThreadContext>
  );
}
