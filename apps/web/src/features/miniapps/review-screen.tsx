import { useInfiniteQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { miniAppKeys } from "@tokuly/core/miniapps";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { Card, Chip } from "#/components/ui/card";
import { TextArea } from "#/components/ui/field";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { useStaffAccess } from "#/features/admin";
import { EmojiText } from "#/features/emoji";
import { errorMessage } from "#/lib/errors";
import { useMe } from "#/stores/session";
import { type dev, NS, reviewClient } from "./api";
import { AppIcon } from "./parts";

/**
 * The staff console's mini-app review: manifests waiting, longest first.
 * A decision names the revision it was made on; one the owner changed since
 * comes back as outdated and is read again. Nobody decides on their own app.
 */
export function ReviewScreen() {
  const { t } = useTranslation(NS);
  const access = useStaffAccess();
  return (
    <div className="h-full overflow-y-auto">
      <header className="glass sticky top-0 z-20 border-b-2 border-line">
        <div className="flex h-14 items-center gap-2 px-4">
          <h1 className="min-w-0 flex-1 truncate text-lg font-black">{t("review.title")}</h1>
        </div>
      </header>
      <div className="mx-auto max-w-3xl px-4 pt-4 pb-32 md:pb-10">
        {access.moderator ? (
          <Queue />
        ) : (
          <EmptyState emoji="🔐" title={t("review.gateTitle")} body={t("review.gateBody")} />
        )}
      </div>
    </div>
  );
}

function Queue() {
  const { t } = useTranslation(NS);
  const queue = useInfiniteQuery({
    queryKey: miniAppKeys.queue,
    queryFn: ({ pageParam }) =>
      reviewClient().listMiniAppReviews({ page: { limit: 20, cursor: pageParam } }),
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
  });
  if (queue.isPending) return <Skeleton className="h-48" />;
  if (queue.isError)
    return <EmptyState emoji="😵" title={errorMessage(queue.error, { namespace: NS })} />;
  const reviews = queue.data.pages.flatMap((page) => page.reviews);
  if (reviews.length === 0)
    return <EmptyState emoji="🎉" title={t("review.emptyTitle")} body={t("review.emptyBody")} />;
  return (
    <div className="flex flex-col gap-4">
      {reviews.map((review) => (
        <ReviewCard key={`${review.appId}:${review.revision}`} review={review} />
      ))}
      <LoadMore query={queue} />
    </div>
  );
}

function ReviewCard({ review }: { review: dev.MiniAppReview }) {
  const { t } = useTranslation(NS);
  const queryClient = useQueryClient();
  const me = useMe();
  const [note, setNote] = useState("");
  const draft = review.draft;
  const owner = review.app?.owner;
  const own = owner?.id === me?.user?.id;
  const decide = useMutation({
    mutationFn: (approve: boolean) =>
      reviewClient().reviewMiniApp({
        appId: review.appId,
        revision: review.revision,
        approve,
        note: note.trim(),
      }),
    onSuccess: (_, approve) => {
      toast.success(approve ? t("review.approved") : t("review.rejected"));
      void queryClient.invalidateQueries({ queryKey: miniAppKeys.queue });
    },
    onError: (error) => {
      toast.error(errorMessage(error, { namespace: NS }));
      void queryClient.invalidateQueries({ queryKey: miniAppKeys.queue });
    },
  });
  if (!draft) return null;
  return (
    <Card className="flex flex-col gap-3 p-4" data-testid="mini-app-review">
      <div className="flex items-center gap-3">
        <AppIcon icon={draft.icon} name={draft.name} className="size-12" />
        <div className="min-w-0 flex-1">
          <p className="truncate text-lg font-black">
            <EmojiText text={draft.name} />
          </p>
          <p className="flex items-center gap-1.5 truncate text-sm text-ink-soft">
            {owner && <Avatar user={owner} size="xs" />}@{owner?.handle} ·{" "}
            <EmojiText text={review.app?.name ?? ""} />
          </p>
        </div>
        {review.live && <Chip>{t("review.update")}</Chip>}
      </div>
      <dl className="grid gap-2 text-sm">
        <div>
          <dt className="text-xs font-bold text-ink-faint">{t("review.launchUrl")}</dt>
          <dd className="font-mono text-xs break-all">{draft.launchUrl}</dd>
          {review.live && review.live.launchUrl !== draft.launchUrl && (
            <dd className="font-mono text-xs break-all text-ink-faint line-through">
              {review.live.launchUrl}
            </dd>
          )}
        </div>
        <div>
          <dt className="text-xs font-bold text-ink-faint">{t("review.scopes")}</dt>
          <dd>
            {draft.scopes.length === 0
              ? t("review.noScopes")
              : draft.scopes.map((scope) => (
                  <Chip key={scope} className="mr-1.5 font-mono">
                    {scope}
                  </Chip>
                ))}
          </dd>
        </div>
        {review.app?.description && (
          <div>
            <dt className="text-xs font-bold text-ink-faint">{t("review.description")}</dt>
            <dd className="text-ink-soft">
              <EmojiText text={review.app.description} />
            </dd>
          </div>
        )}
      </dl>
      {own ? (
        <p className="rounded-2xl bg-surface-2 px-3.5 py-3 text-sm font-bold text-ink-soft">
          {t("review.own")}
        </p>
      ) : (
        <>
          <TextArea
            label={t("review.note")}
            hint={t("review.noteHint")}
            value={note}
            onChange={(event) => setNote(event.target.value)}
            maxLength={500}
          />
          <div className="flex flex-wrap gap-2">
            <Button
              loading={decide.isPending && decide.variables === true}
              disabled={decide.isPending}
              onClick={() => decide.mutate(true)}
            >
              {t("review.approve")}
            </Button>
            <Button
              variant="secondary"
              loading={decide.isPending && decide.variables === false}
              disabled={decide.isPending || note.trim().length === 0}
              onClick={() => decide.mutate(false)}
            >
              {t("review.reject")}
            </Button>
          </div>
        </>
      )}
    </Card>
  );
}
