import { useInfiniteQuery, useMutation } from "@tanstack/react-query";
import { emoji as emojiApi, toDate } from "@tokuly/api";
import { emojiKeys, shortcode, toCatalogEmoji } from "@tokuly/core/emoji";
import { BadgeCheck, Check, Pencil, X } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Chip } from "#/components/ui/card";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { errorMessage } from "#/lib/errors";
import { EmojiImage } from "../emoji";
import { ThemePreview } from "../parts";
import { EmojiEditDialog, ReasonDialog } from "./dialogs";
import { emojiAdminClient, UserLine, useAdminRefresh } from "./shared";

export function useReviewQueue() {
  return useInfiniteQuery({
    queryKey: emojiKeys.adminQueue,
    queryFn: ({ pageParam }) =>
      emojiAdminClient().listReviewQueue({ page: { limit: 20, cursor: pageParam } }),
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
  });
}

/** The review queue: what people submitted, oldest first, each with everything needed to decide. */
export function ReviewQueue() {
  const { t } = useTranslation("emoji");
  const queue = useReviewQueue();
  if (queue.isPending) {
    return (
      <div className="grid gap-4 lg:grid-cols-2">
        <Skeleton className="h-64" />
        <Skeleton className="h-64" />
      </div>
    );
  }
  if (queue.isError)
    return (
      <EmptyState
        emoji="😵‍💫"
        title={t("browse.error")}
        body={errorMessage(queue.error, { namespace: "emoji" })}
      />
    );
  const items = queue.data.pages.flatMap((page) => page.emojis);
  if (items.length === 0)
    return <EmptyState emoji="🍵" title={t("admin.queueEmpty")} body={t("admin.queueEmptyBody")} />;
  return (
    <>
      <p className="mb-3 text-sm font-bold text-ink-soft">
        {t("admin.waiting", { count: queue.data.pages[0]?.pendingCount ?? items.length })}
      </p>
      <div className="grid items-start gap-4 lg:grid-cols-2 2xl:grid-cols-3">
        {items.map((item) => item.emoji && <ReviewCard key={item.emoji.id} detail={item} />)}
      </div>
      <LoadMore query={queue} />
    </>
  );
}

const ACTION_KEYS: Record<emojiApi.ReviewAction, string> = {
  [emojiApi.ReviewAction.UNSPECIFIED]: "edited",
  [emojiApi.ReviewAction.SUBMITTED]: "submitted",
  [emojiApi.ReviewAction.APPROVED]: "approved",
  [emojiApi.ReviewAction.REJECTED]: "rejected",
  [emojiApi.ReviewAction.DISABLED]: "disabled",
  [emojiApi.ReviewAction.ENABLED]: "enabled",
  [emojiApi.ReviewAction.ADDED]: "added",
  [emojiApi.ReviewAction.IMPORTED]: "imported",
  [emojiApi.ReviewAction.EDITED]: "edited",
  [emojiApi.ReviewAction.TAKEN_DOWN]: "takenDown",
};

/** What happened to an emoji so far, newest first. */
export function History({ records }: { records: emojiApi.ReviewRecord[] }) {
  const { t, i18n } = useTranslation("emoji");
  if (records.length === 0) return null;
  const format = new Intl.DateTimeFormat(i18n.language, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
  return (
    <ol className="flex flex-col gap-1 text-xs text-ink-soft">
      {records.map((record, index) => {
        const at = toDate(record.createdAt);
        return (
          // Records have no ID and never reorder.
          <li key={index} className="flex flex-wrap gap-x-2">
            <span className="font-bold text-ink">
              {t(`admin.history.${ACTION_KEYS[record.action]}`)}
            </span>
            {record.actor && <span>@{record.actor.handle}</span>}
            {at && <span className="text-ink-faint">{format.format(at)}</span>}
            {record.note && <span className="w-full break-words">{record.note}</span>}
          </li>
        );
      })}
    </ol>
  );
}

function ReviewCard({ detail }: { detail: emojiApi.EmojiDetail }) {
  const { t } = useTranslation("emoji");
  const refresh = useAdminRefresh();
  const entry = toCatalogEmoji(detail.emoji as emojiApi.CustomEmoji);
  const code = shortcode(entry.name);
  const [rejecting, setRejecting] = useState(false);
  const [editing, setEditing] = useState(false);

  const onError = (error: unknown) => {
    toast.error(errorMessage(error, { namespace: "emoji" }));
    // Someone else may have decided meanwhile: show the queue as it is.
    void refresh();
  };
  const approve = useMutation({
    mutationFn: () => emojiAdminClient().approveEmoji({ emojiId: entry.id }),
    onSuccess: async () => {
      toast.success(t("admin.approved", { code }));
      await refresh();
    },
    onError,
  });
  const reject = useMutation({
    mutationFn: (reason: string) => emojiAdminClient().rejectEmoji({ emojiId: entry.id, reason }),
    onSuccess: async () => {
      setRejecting(false);
      toast.success(t("admin.rejected", { code }));
      await refresh();
    },
    onError,
  });
  const trust = useMutation({
    mutationFn: () =>
      emojiAdminClient().setSubmitterTrusted({ userId: detail.author?.id ?? "", trusted: true }),
    onSuccess: async () => {
      toast.success(t("admin.trustedDone", { name: detail.author?.displayName ?? "" }));
      await refresh();
    },
    onError,
  });

  return (
    <article
      data-testid="review-card"
      aria-label={code}
      className="rounded-2xl border-2 border-line bg-surface p-4"
    >
      <div className="flex items-center gap-3">
        <span className="grid size-16 shrink-0 place-items-center rounded-2xl bg-surface-2">
          <EmojiImage emoji={entry} size={48} reveal className="size-12" />
        </span>
        <div className="min-w-0 flex-1">
          <p className="truncate font-mono text-lg font-bold">{code}</p>
          <div className="mt-1 flex flex-wrap gap-1.5">
            {entry.category && <Chip>{entry.category}</Chip>}
            {entry.animated && <Chip>{t("flags.animated")}</Chip>}
            {entry.sensitive && (
              <Chip className="bg-coral text-[#17122b]">{t("flags.sensitive")}</Chip>
            )}
          </div>
        </div>
      </div>
      <ThemePreview className="mt-3">
        <p>
          {t("form.previewBefore")}
          <EmojiImage emoji={entry} reveal />
          {t("form.previewAfter")}
        </p>
      </ThemePreview>
      <dl className="mt-3 flex flex-col gap-1.5 text-sm">
        <div className="flex gap-2">
          <dt className="w-20 shrink-0 text-xs font-black text-ink-faint">{t("detail.author")}</dt>
          <dd className="min-w-0 flex-1 truncate">
            <UserLine user={detail.author} />
          </dd>
        </div>
        <div className="flex gap-2">
          <dt className="w-20 shrink-0 text-xs font-black text-ink-faint">
            {t("fields.aliasesShort")}
          </dt>
          <dd className="min-w-0 flex-1 break-words">{entry.aliases.join(" · ") || "—"}</dd>
        </div>
        <div className="flex gap-2">
          <dt className="w-20 shrink-0 text-xs font-black text-ink-faint">
            {t("fields.licenseShort")}
          </dt>
          <dd className="min-w-0 flex-1 whitespace-pre-wrap break-words">
            {detail.license || "—"}
          </dd>
        </div>
      </dl>
      {detail.history.length > 1 && (
        <div className="mt-3 rounded-xl bg-surface-2 px-3 py-2">
          <History records={detail.history} />
        </div>
      )}
      <div className="mt-4 flex flex-wrap gap-2">
        <Button
          variant="lime"
          size="sm"
          icon={<Check className="size-4" />}
          loading={approve.isPending}
          onClick={() => approve.mutate()}
        >
          {t("admin.approve")}
        </Button>
        <Button
          variant="secondary"
          size="sm"
          icon={<X className="size-4" />}
          onClick={() => setRejecting(true)}
        >
          {t("admin.reject")}
        </Button>
        <Button
          variant="ghost"
          size="sm"
          icon={<Pencil className="size-4" />}
          onClick={() => setEditing(true)}
        >
          {t("admin.edit")}
        </Button>
        {detail.author && !detail.author.deleted && (
          <Button
            variant="ghost"
            size="sm"
            icon={<BadgeCheck className="size-4" />}
            loading={trust.isPending}
            onClick={() => trust.mutate()}
          >
            {t("admin.trustAuthor")}
          </Button>
        )}
      </div>
      <ReasonDialog
        open={rejecting}
        onOpenChange={setRejecting}
        title={t("admin.rejectTitle", { code })}
        confirmLabel={t("admin.reject")}
        loading={reject.isPending}
        onConfirm={(reason) => reject.mutate(reason)}
      />
      {editing && <EmojiEditDialog detail={detail} open onOpenChange={setEditing} />}
    </article>
  );
}
