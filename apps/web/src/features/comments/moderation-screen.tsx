import {
  type InfiniteData,
  useInfiniteQuery,
  useMutation,
  useQueryClient,
} from "@tanstack/react-query";
import { common, engagement } from "@tokuly/api";
import {
  Ban,
  Check,
  EyeOff,
  Flag,
  MoreVertical,
  Search,
  Trash2,
  UserSearch,
  X,
} from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { LoadMore } from "#/components/ui/load-more";
import { Menu } from "#/components/ui/menu";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { Tabs } from "#/components/ui/tabs";
import { ReportDialog } from "#/features/engagement";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { useTranslation } from "#/lib/use-translation";
import {
  commentClient,
  commentKeys,
  moderationClient,
  type ReceivedFilter,
  useHeldCount,
} from "./api";
import { authorName } from "./author";
import { ContextRow } from "./context-row";
import { BlockedCommenters, Rules } from "./moderation-rules";
import type { ModerationTab } from "./moderation-search";
import { useReviewQueueRealtime } from "./realtime";

const { CommentStatus } = engagement;

type ReceivedPages = InfiniteData<engagement.ListReceivedCommentsResponse>;

/**
 * `/settings/comments`: everything about comments that belongs to the
 * signed-in person — the ones waiting for their review, all they received
 * on their own content, the ones they wrote, their screening rules and the
 * people they stopped from commenting.
 */
export function CommentModerationScreen({
  tab,
  onTabChange,
}: {
  tab: ModerationTab;
  onTabChange(tab: ModerationTab): void;
}) {
  const { t } = useTranslation("comments");
  const held = useHeldCount();
  useReviewQueueRealtime();
  // Five tabs do not fit a phone: the open one is brought into view.
  const tabs = useRef<HTMLDivElement>(null);
  // re-run whenever another tab opens.
  useEffect(() => {
    tabs.current
      ?.querySelector('[aria-selected="true"]')
      ?.scrollIntoView({ block: "nearest", inline: "center" });
  }, [tab]);

  return (
    <Page title={t("moderation.title")} back="/settings">
      <div ref={tabs}>
        <Tabs<ModerationTab>
          className="mb-4"
          value={tab}
          onChange={onTabChange}
          items={[
            { value: "queue", label: t("moderation.tabs.queue"), badge: held.data },
            { value: "received", label: t("moderation.tabs.received") },
            { value: "mine", label: t("moderation.tabs.mine") },
            { value: "rules", label: t("moderation.tabs.rules") },
            { value: "blocked", label: t("moderation.tabs.blocked") },
          ]}
        />
      </div>
      {tab === "queue" && <Received key="queue" queue />}
      {tab === "received" && <Received key="received" />}
      {tab === "mine" && <MyComments />}
      {tab === "rules" && <Rules />}
      {tab === "blocked" && <BlockedCommenters />}
    </Page>
  );
}

function RowsSkeleton() {
  return (
    <div className="flex flex-col gap-3" aria-busy>
      {[0, 1, 2].map((row) => (
        <Skeleton key={row} className="h-32 rounded-card" />
      ))}
    </div>
  );
}

const STATUS_FILTERS = [
  { value: CommentStatus.UNSPECIFIED, key: "moderation.filter.all" },
  { value: CommentStatus.PUBLISHED, key: "moderation.filter.published" },
  { value: CommentStatus.HELD, key: "moderation.filter.held" },
  { value: CommentStatus.HIDDEN, key: "moderation.filter.hidden" },
] as const;

/** Comments other people wrote on the viewer's content: the review queue (`queue`) or all of them with filters. */
function Received({ queue }: { queue?: boolean }) {
  const { t } = useTranslation("comments");
  const queryClient = useQueryClient();
  const [status, setStatus] = useState<engagement.CommentStatus>(
    queue ? CommentStatus.HELD : CommentStatus.UNSPECIFIED,
  );
  const [search, setSearch] = useState("");
  const [query, setQuery] = useState("");
  const [authorOf, setAuthorOf] = useState<{ commentId: string; name: string } | null>(null);
  const [reporting, setReporting] = useState<engagement.Comment | null>(null);
  const [confirming, setConfirming] = useState<{
    kind: "delete" | "block";
    comment: engagement.Comment;
  } | null>(null);

  useEffect(() => {
    const timer = setTimeout(() => setQuery(search.trim()), 300);
    return () => clearTimeout(timer);
  }, [search]);

  const filter: ReceivedFilter = { status, query, authorOfCommentId: authorOf?.commentId ?? "" };
  const list = useInfiniteQuery({
    queryKey: commentKeys.receivedList(filter),
    queryFn: ({ pageParam }) =>
      moderationClient().listReceivedComments({
        ...filter,
        page: { limit: 30, cursor: pageParam },
      }),
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
  });

  /** Updates (or, with `null`, drops) one row in every cached list of received comments. */
  const patchRow = (commentId: string, next: engagement.Comment | null) => {
    queryClient.setQueriesData<ReceivedPages>({ queryKey: commentKeys.received }, (data) =>
      data
        ? {
            ...data,
            pages: data.pages.map((page) => ({
              ...page,
              comments: page.comments.flatMap((entry) =>
                entry.comment?.id !== commentId
                  ? [entry]
                  : next
                    ? [{ ...entry, comment: next }]
                    : [],
              ),
            })),
          }
        : data,
    );
  };
  const refreshCounts = () => {
    void queryClient.invalidateQueries({ queryKey: commentKeys.heldCount });
    // Threads on screen elsewhere show the change the next time they are read.
    void queryClient.invalidateQueries({ queryKey: [...commentKeys.all, "thread"] });
  };
  const onError = (error: unknown) => toast.error(errorMessage(error, { namespace: "comments" }));

  const approve = useMutation({
    mutationFn: (comment: engagement.Comment) =>
      moderationClient().approveComment({ commentId: comment.id }),
    onSuccess: ({ comment }, original) => {
      // In the queue an approved comment is done with; elsewhere it stays, now published.
      patchRow(
        original.id,
        status === CommentStatus.UNSPECIFIED || status === CommentStatus.PUBLISHED
          ? (comment ?? null)
          : null,
      );
      refreshCounts();
      toast.success(t("toast.approved"));
    },
    onError,
  });
  const hide = useMutation({
    mutationFn: (comment: engagement.Comment) =>
      moderationClient().hideComment({ commentId: comment.id }),
    onSuccess: ({ comment }, original) => {
      patchRow(
        original.id,
        status === CommentStatus.UNSPECIFIED || status === CommentStatus.HIDDEN
          ? (comment ?? null)
          : null,
      );
      refreshCounts();
      toast.success(t("toast.hidden"));
    },
    onError,
  });
  const remove = useMutation({
    mutationFn: (comment: engagement.Comment) =>
      commentClient().deleteComment({ commentId: comment.id }),
    onSuccess: (_response, original) => {
      setConfirming(null);
      patchRow(original.id, null);
      refreshCounts();
      toast.success(t("toast.deleted"));
    },
    onError: (error) => {
      setConfirming(null);
      onError(error);
    },
  });
  const block = useMutation({
    mutationFn: (comment: engagement.Comment) =>
      moderationClient().blockCommenter({ target: { case: "commentId", value: comment.id } }),
    onSuccess: (_response, original) => {
      setConfirming(null);
      // Their comments leave these lists altogether.
      void queryClient.invalidateQueries({ queryKey: commentKeys.received });
      void queryClient.invalidateQueries({ queryKey: commentKeys.blocked });
      refreshCounts();
      toast.success(t("toast.blocked", { name: authorName(original.author) }));
    },
    onError: (error) => {
      setConfirming(null);
      onError(error);
    },
  });

  const entries = list.data?.pages.flatMap((page) => page.comments) ?? [];
  const filtered =
    query !== "" || authorOf !== null || (!queue && status !== CommentStatus.UNSPECIFIED);

  return (
    <div className="flex flex-col gap-3">
      <p className="px-1 text-sm leading-relaxed text-ink-soft">
        {queue ? t("moderation.queueLead") : t("moderation.receivedLead")}
      </p>
      {!queue && (
        <>
          <label className="flex h-12 items-center gap-2 rounded-control border-2 border-line bg-surface px-4 focus-within:border-brand">
            <Search className="size-5 shrink-0 text-ink-faint" />
            <input
              value={search}
              onChange={(event) => setSearch(event.target.value)}
              maxLength={100}
              placeholder={t("moderation.searchPlaceholder")}
              aria-label={t("moderation.searchPlaceholder")}
              className="min-w-0 flex-1 bg-transparent outline-none placeholder:text-ink-faint"
            />
          </label>
          <fieldset className="scrollbar-none -mx-4 flex min-w-0 gap-2 overflow-x-auto px-4">
            <legend className="sr-only">{t("moderation.filter.label")}</legend>
            {STATUS_FILTERS.map((option) => (
              <button
                key={option.value}
                type="button"
                aria-pressed={status === option.value}
                className={cn(
                  "h-9 shrink-0 rounded-full border-2 px-4 text-sm font-bold",
                  status === option.value
                    ? "border-ink bg-ink text-bg"
                    : "border-line bg-surface text-ink-soft",
                )}
                onClick={() => setStatus(option.value)}
              >
                {t(option.key)}
              </button>
            ))}
          </fieldset>
        </>
      )}
      {authorOf && (
        <p className="flex items-center gap-2 self-start rounded-full bg-brand-soft py-1 pr-1 pl-4 text-sm font-black text-brand-strong">
          {t("moderation.byAuthor", { name: authorOf.name })}
          <button
            type="button"
            aria-label={t("moderation.clearAuthor")}
            className="grid size-7 place-items-center rounded-full hover:bg-surface"
            onClick={() => setAuthorOf(null)}
          >
            <X className="size-4" />
          </button>
        </p>
      )}

      {list.isPending ? (
        <RowsSkeleton />
      ) : list.isError ? (
        <EmptyState emoji="😵‍💫" title={errorMessage(list.error, { namespace: "comments" })} />
      ) : entries.length === 0 ? (
        filtered ? (
          <EmptyState emoji="🔍" title={t("moderation.emptyFiltered")} />
        ) : queue ? (
          <EmptyState
            emoji="🎉"
            title={t("moderation.emptyQueue")}
            body={t("moderation.emptyQueueBody")}
          />
        ) : (
          <EmptyState
            emoji="💌"
            title={t("moderation.emptyReceived")}
            body={t("moderation.emptyReceivedBody")}
          />
        )
      ) : (
        <>
          {entries.map((entry) => {
            const comment = entry.comment;
            if (!comment) return null;
            const published = comment.status === CommentStatus.PUBLISHED;
            return (
              <ContextRow
                key={comment.id}
                entry={entry}
                showAuthor
                actions={
                  <>
                    {published ? (
                      <Button
                        size="sm"
                        variant="secondary"
                        icon={<EyeOff className="size-4" />}
                        loading={hide.isPending && hide.variables?.id === comment.id}
                        onClick={() => hide.mutate(comment)}
                      >
                        {t("actions.hide")}
                      </Button>
                    ) : (
                      <Button
                        size="sm"
                        variant="lime"
                        icon={<Check className="size-4" />}
                        loading={approve.isPending && approve.variables?.id === comment.id}
                        onClick={() => approve.mutate(comment)}
                      >
                        {t("actions.approve")}
                      </Button>
                    )}
                    <Button
                      size="sm"
                      variant="ghost"
                      className="text-danger"
                      icon={<Trash2 className="size-4" />}
                      onClick={() => setConfirming({ kind: "delete", comment })}
                    >
                      {t("actions.delete")}
                    </Button>
                    <Menu
                      trigger={
                        <Button size="icon-sm" variant="ghost" aria-label={t("actions.more")}>
                          <MoreVertical className="size-4" />
                        </Button>
                      }
                      actions={[
                        {
                          label: t("moderation.onlyThisAuthor"),
                          icon: <UserSearch />,
                          onSelect: () =>
                            setAuthorOf({
                              commentId: comment.id,
                              name: authorName(comment.author),
                            }),
                        },
                        {
                          label: t("engagement:report.action"),
                          icon: <Flag />,
                          onSelect: () => setReporting(comment),
                        },
                        {
                          label: t("actions.block"),
                          icon: <Ban />,
                          danger: true,
                          onSelect: () => setConfirming({ kind: "block", comment }),
                        },
                      ]}
                    />
                  </>
                }
              />
            );
          })}
          <LoadMore query={list} />
        </>
      )}

      {reporting && (
        <ReportDialog
          subject={{ type: common.SubjectType.COMMENT, id: reporting.id }}
          open
          onOpenChange={(open) => !open && setReporting(null)}
          label={`${authorName(reporting.author)}: ${reporting.text.slice(0, 80)}`}
        />
      )}
      <ConfirmDialog
        open={confirming?.kind === "delete"}
        onOpenChange={(open) => !open && setConfirming(null)}
        title={t("confirm.deleteTitle")}
        description={t("confirm.deleteBody")}
        confirmLabel={t("actions.delete")}
        danger
        loading={remove.isPending}
        onConfirm={() => confirming && remove.mutate(confirming.comment)}
      />
      <ConfirmDialog
        open={confirming?.kind === "block"}
        onOpenChange={(open) => !open && setConfirming(null)}
        title={t("confirm.blockTitle", {
          name: confirming ? authorName(confirming.comment.author) : "",
        })}
        description={t("confirm.blockBody")}
        confirmLabel={t("actions.block")}
        danger
        loading={block.isPending}
        onConfirm={() => confirming && block.mutate(confirming.comment)}
      />
    </div>
  );
}

/** Everything the signed-in person has commented, newest first, each leading to where it was written. */
function MyComments() {
  const { t } = useTranslation("comments");
  const queryClient = useQueryClient();
  const [deleting, setDeleting] = useState<engagement.Comment | null>(null);
  const list = useInfiniteQuery({
    queryKey: commentKeys.mine,
    queryFn: ({ pageParam }) =>
      commentClient().listMyComments({ page: { limit: 30, cursor: pageParam } }),
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
  });
  const remove = useMutation({
    mutationFn: (comment: engagement.Comment) =>
      commentClient().deleteComment({ commentId: comment.id }),
    onSuccess: (_response, comment) => {
      setDeleting(null);
      queryClient.setQueryData<InfiniteData<engagement.ListMyCommentsResponse>>(
        commentKeys.mine,
        (data) =>
          data
            ? {
                ...data,
                pages: data.pages.map((page) => ({
                  ...page,
                  comments: page.comments.filter((entry) => entry.comment?.id !== comment.id),
                })),
              }
            : data,
      );
      void queryClient.invalidateQueries({ queryKey: [...commentKeys.all, "thread"] });
      toast.success(t("toast.deleted"));
    },
    onError: (error) => {
      setDeleting(null);
      toast.error(errorMessage(error, { namespace: "comments" }));
    },
  });
  const entries = list.data?.pages.flatMap((page) => page.comments) ?? [];

  return (
    <div className="flex flex-col gap-3">
      <p className="px-1 text-sm leading-relaxed text-ink-soft">{t("moderation.mineLead")}</p>
      {list.isPending ? (
        <RowsSkeleton />
      ) : list.isError ? (
        <EmptyState emoji="😵‍💫" title={errorMessage(list.error, { namespace: "comments" })} />
      ) : entries.length === 0 ? (
        <EmptyState
          emoji="✍️"
          title={t("moderation.emptyMine")}
          body={t("moderation.emptyMineBody")}
        />
      ) : (
        <>
          {entries.map(
            (entry) =>
              entry.comment && (
                <ContextRow
                  key={entry.comment.id}
                  entry={entry}
                  showAuthor={false}
                  actions={
                    <Button
                      size="sm"
                      variant="ghost"
                      className="text-danger"
                      icon={<Trash2 className="size-4" />}
                      onClick={() => setDeleting(entry.comment ?? null)}
                    >
                      {t("actions.delete")}
                    </Button>
                  }
                />
              ),
          )}
          <LoadMore query={list} />
        </>
      )}
      <ConfirmDialog
        open={deleting !== null}
        onOpenChange={(open) => !open && setDeleting(null)}
        title={t("confirm.deleteTitle")}
        description={t("confirm.deleteBody")}
        confirmLabel={t("actions.delete")}
        danger
        loading={remove.isPending}
        onConfirm={() => deleting && remove.mutate(deleting)}
      />
    </div>
  );
}
