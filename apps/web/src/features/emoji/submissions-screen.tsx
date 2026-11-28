import { useInfiniteQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { emoji as emojiApi, toDate } from "@tokuly/api";
import { emojiKeys, emojiPageParam, shortcode, toCatalogEmoji } from "@tokuly/core/emoji";
import { BadgeCheck, Pencil, Plus, Trash2 } from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { Tabs } from "#/components/ui/tabs";
import { errorMessage } from "#/lib/errors";
import { useTranslation } from "#/lib/use-translation";
import { submissionClient } from "./api";
import { EmojiImage } from "./emoji";
import { StateChip } from "./parts";
import { useSubmitterProfile } from "./submit-screen";

const FILTERS = ["all", "pending", "approved", "rejected", "disabled"] as const;
type Filter = (typeof FILTERS)[number];
const FILTER_STATES: Record<Filter, emojiApi.EmojiState> = {
  all: emojiApi.EmojiState.UNSPECIFIED,
  pending: emojiApi.EmojiState.PENDING,
  approved: emojiApi.EmojiState.APPROVED,
  rejected: emojiApi.EmojiState.REJECTED,
  disabled: emojiApi.EmojiState.DISABLED,
};

/** "My submissions": every emoji the person sent in, with where each stands and what the moderators said. */
export function SubmissionsScreen() {
  const { t } = useTranslation("emoji");
  const [filter, setFilter] = useState<Filter>("all");
  const profile = useSubmitterProfile();
  const state = FILTER_STATES[filter];
  const list = useInfiniteQuery({
    queryKey: emojiKeys.submissionList(state),
    queryFn: ({ pageParam }) =>
      submissionClient().listMySubmissions({ state, page: { limit: 30, cursor: pageParam } }),
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
  });
  const items = list.data?.pages.flatMap((page) => page.emojis) ?? [];
  const limits = profile.data;

  return (
    <Page
      title={t("submissions.title")}
      back="/emoji"
      actions={
        <Link
          to="/emoji/new-emoji"
          className="pressable sticker inline-flex h-9 items-center gap-1.5 rounded-full bg-brand px-4 text-sm font-bold text-on-brand"
        >
          <Plus className="size-4" />
          {t("submit.action")}
        </Link>
      }
    >
      {limits && (
        <div className="mb-4 grid grid-cols-2 gap-2 text-center">
          <div className="rounded-2xl bg-surface px-3 py-3">
            <p className="text-2xl font-black">{limits.approvedCount}</p>
            <p className="text-xs font-bold text-ink-soft">{t("submissions.inCatalog")}</p>
          </div>
          <div className="rounded-2xl bg-surface px-3 py-3">
            {limits.trusted ? (
              <>
                <p className="grid h-8 place-items-center text-success">
                  <BadgeCheck className="size-7" />
                </p>
                <p className="text-xs font-bold text-ink-soft">{t("submissions.trusted")}</p>
              </>
            ) : (
              <>
                <p className="text-2xl font-black">
                  {limits.pendingCount}
                  <span className="text-sm text-ink-faint"> / {limits.maxPending}</span>
                </p>
                <p className="text-xs font-bold text-ink-soft">{t("submissions.waiting")}</p>
              </>
            )}
          </div>
        </div>
      )}
      <Tabs<Filter>
        value={filter}
        onChange={setFilter}
        items={FILTERS.map((value) => ({
          value,
          label: t(value === "all" ? "submissions.all" : `state.${value}`),
        }))}
      />
      <div className="mt-4 flex flex-col gap-3">
        {list.isPending ? (
          <>
            <Skeleton className="h-24" />
            <Skeleton className="h-24" />
          </>
        ) : list.isError ? (
          <EmptyState
            emoji="😵‍💫"
            title={t("browse.error")}
            body={errorMessage(list.error, { namespace: "emoji" })}
          />
        ) : items.length === 0 ? (
          <EmptyState
            emoji="🎨"
            title={t(filter === "all" ? "submissions.empty" : "submissions.emptyFiltered")}
            body={filter === "all" ? t("submissions.emptyBody") : undefined}
            action={
              filter === "all" ? (
                <Link
                  to="/emoji/new-emoji"
                  className="pressable sticker inline-flex h-11 items-center rounded-full bg-brand px-5 font-bold text-on-brand"
                >
                  {t("submit.action")}
                </Link>
              ) : undefined
            }
          />
        ) : (
          items.map((item) => item.emoji && <SubmissionCard key={item.emoji.id} detail={item} />)
        )}
        <LoadMore query={list} />
      </div>
    </Page>
  );
}

function SubmissionCard({ detail }: { detail: emojiApi.EmojiDetail }) {
  const { t, i18n } = useTranslation("emoji");
  const queryClient = useQueryClient();
  const [confirming, setConfirming] = useState(false);
  const entry = toCatalogEmoji(detail.emoji as emojiApi.CustomEmoji);
  const approved = detail.state === emojiApi.EmojiState.APPROVED;
  const date = toDate(detail.createdAt);
  const withdraw = useMutation({
    mutationFn: () => submissionClient().withdrawSubmission({ emojiId: entry.id }),
    onSuccess: async () => {
      setConfirming(false);
      toast.success(t("submissions.withdrawn"));
      await queryClient.invalidateQueries({ queryKey: emojiKeys.submissions });
      await queryClient.invalidateQueries({ queryKey: emojiKeys.submitter });
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "emoji" })),
  });

  return (
    <article data-testid="submission" className="rounded-card bg-surface p-4">
      <div className="flex items-center gap-3">
        <Link
          to="/emoji/$name"
          params={{ name: approved ? emojiPageParam(entry.name) : entry.id }}
          className="grid size-16 shrink-0 place-items-center rounded-2xl bg-surface-2"
          aria-label={shortcode(entry.name)}
        >
          <EmojiImage emoji={entry} size={48} reveal className="size-12" />
        </Link>
        <div className="min-w-0 flex-1">
          <p className="truncate font-mono font-bold">{shortcode(entry.name)}</p>
          <div className="mt-1 flex flex-wrap items-center gap-1.5">
            <StateChip state={detail.state} />
            {date && (
              <span className="text-xs text-ink-faint">
                {new Intl.DateTimeFormat(i18n.language, { month: "short", day: "numeric" }).format(
                  date,
                )}
              </span>
            )}
          </div>
        </div>
      </div>
      {detail.reviewNote && !approved && (
        <p className="mt-3 rounded-2xl bg-surface-2 px-3 py-2 text-sm">
          <span className="block text-xs font-black text-ink-faint">{t("submissions.note")}</span>
          <span className="whitespace-pre-wrap break-words">{detail.reviewNote}</span>
        </p>
      )}
      {detail.state === emojiApi.EmojiState.PENDING && (
        <p className="mt-3 text-sm text-ink-soft">{t("submissions.pendingBody")}</p>
      )}
      {detail.canEdit && (
        <div className="mt-3 flex flex-wrap gap-2">
          <Link
            to="/emoji/submissions/$emojiId"
            params={{ emojiId: entry.id }}
            className="pressable sticker inline-flex h-9 items-center gap-1.5 rounded-full bg-surface px-4 text-sm font-bold"
          >
            <Pencil className="size-4" />
            {t(
              detail.state === emojiApi.EmojiState.REJECTED
                ? "submissions.fix"
                : "submissions.edit",
            )}
          </Link>
          <Button
            variant="ghost"
            size="sm"
            className="text-danger"
            icon={<Trash2 className="size-4" />}
            onClick={() => setConfirming(true)}
          >
            {t("submissions.withdraw")}
          </Button>
        </div>
      )}
      <ConfirmDialog
        open={confirming}
        onOpenChange={setConfirming}
        title={t("submissions.withdrawTitle", { code: shortcode(entry.name) })}
        description={t("submissions.withdrawBody")}
        confirmLabel={t("submissions.withdraw")}
        danger
        loading={withdraw.isPending}
        onConfirm={() => withdraw.mutate()}
      />
    </article>
  );
}
