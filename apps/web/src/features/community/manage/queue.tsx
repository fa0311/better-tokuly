import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { community, errorReason } from "@tokuly/api";
import { can, communityKeys } from "@tokuly/core/community";
import { Eye, History } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Chip } from "#/components/ui/card";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { EmojiText } from "#/features/emoji";
import { errorMessage } from "#/lib/errors";
import { moderationClient, NS, useReviewQueue } from "../api";
import { ContentHistoryDialog, ReasonDialog, RevealAuthorDialog } from "../mod-dialogs";
import { Notice, PostAuthor, PostBody, PostImages } from "../parts";

/** Hidden posts waiting for a decision: put back, or removed for good. */
export function QueueSection({ community: c }: { community: community.Community }) {
  const { t } = useTranslation(NS);
  const queue = useReviewQueue(c.id);
  if (queue.isPending) return <Skeleton className="h-40" />;
  return (
    <div className="flex flex-col gap-3">
      <Notice>{t("manage.queue.hint")}</Notice>
      {queue.items?.length === 0 ? (
        <EmptyState emoji="🧹" title={t("manage.queue.empty")} body={t("manage.queue.emptyBody")} />
      ) : (
        <>
          {queue.items?.map((item) => (
            <QueueItem
              key={item.post.case === "thread" ? item.post.value.id : item.post.value?.id}
              community={c}
              item={item}
            />
          ))}
          <LoadMore query={queue} />
        </>
      )}
    </div>
  );
}

function QueueItem({
  community: c,
  item,
}: {
  community: community.Community;
  item: community.ReviewItem;
}) {
  const { t } = useTranslation(NS);
  const queryClient = useQueryClient();
  const [removing, setRemoving] = useState(false);
  const [revealing, setRevealing] = useState(false);
  const [history, setHistory] = useState(false);
  const post = item.post.value;
  const isThread = item.post.case === "thread";
  const ref: community.PostRef["post"] = isThread
    ? { case: "threadId", value: post?.id ?? "" }
    : { case: "replyId", value: post?.id ?? "" };
  const threadId =
    item.post.case === "thread" ? item.post.value.id : (item.post.value?.threadId ?? "");

  const review = useMutation({
    mutationFn: (input: { decision: community.ReviewDecision; reason: string }) =>
      moderationClient().reviewPost({ post: { post: ref }, ...input }),
    onSuccess: (_, { decision }) => {
      setRemoving(false);
      void queryClient.invalidateQueries({ queryKey: communityKeys.queue(c.id) });
      void queryClient.invalidateQueries({ queryKey: communityKeys.threadLists(c.id) });
      void queryClient.invalidateQueries({ queryKey: communityKeys.communities });
      toast(
        decision === community.ReviewDecision.RESTORE
          ? t("manage.queue.restored")
          : t("manage.queue.removed"),
      );
    },
    onError: (error) =>
      toast.error(
        errorReason(error) === "CONFLICT_OF_INTEREST"
          ? t("manage.queue.ownPost")
          : errorMessage(error, { namespace: NS }),
      ),
  });
  if (!post) return null;
  return (
    <article className="rounded-card bg-surface p-4">
      <div className="flex flex-wrap items-center gap-1.5">
        <Chip className="bg-coral text-white">
          {item.reason === community.HiddenReason.MUTED_WORD
            ? t("manage.queue.mutedWord")
            : t("manage.queue.reports", { count: item.reportCount })}
        </Chip>
        <Chip>{isThread ? t("manage.queue.thread") : t("manage.queue.reply")}</Chip>
      </div>
      <Link
        to="/g/$handle/t/$threadId"
        params={{ handle: c.handle, threadId }}
        className="mt-2 block break-words text-sm font-bold text-brand"
      >
        <EmojiText text={item.threadTitle} />
      </Link>
      <div className="mt-2">
        <PostAuthor
          size="sm"
          author={post.author}
          pseudonym={post.pseudonym}
          mine={post.viewer?.mine}
          createdAtMs={Number(post.createdAt?.seconds ?? 0n) * 1000}
        />
      </div>
      <PostBody body={post.body} mentions={post.mentions} className="mt-2" />
      {post.images.length > 0 && (
        <div className="mt-2">
          <PostImages images={post.images} />
        </div>
      )}
      <div className="mt-3 flex flex-wrap justify-end gap-2">
        <Button
          variant="ghost"
          size="sm"
          icon={<History className="size-4" />}
          onClick={() => setHistory(true)}
        >
          {t("history.action")}
        </Button>
        {post.pseudonym && can(c, community.Permission.REVEAL_AUTHOR) && (
          <Button
            variant="ghost"
            size="sm"
            icon={<Eye className="size-4" />}
            onClick={() => setRevealing(true)}
          >
            {t("reveal.action")}
          </Button>
        )}
        <Button
          variant="danger"
          size="sm"
          disabled={review.isPending}
          onClick={() => setRemoving(true)}
        >
          {t("manage.queue.remove")}
        </Button>
        <Button
          variant="lime"
          size="sm"
          disabled={review.isPending}
          onClick={() => review.mutate({ decision: community.ReviewDecision.RESTORE, reason: "" })}
        >
          {t("manage.queue.restore")}
        </Button>
      </div>
      <ReasonDialog
        open={removing}
        onOpenChange={setRemoving}
        title={t("manage.queue.removeTitle")}
        description={t("manage.queue.removeBody")}
        confirmLabel={t("manage.queue.remove")}
        loading={review.isPending}
        onConfirm={(reason) => review.mutate({ decision: community.ReviewDecision.REMOVE, reason })}
      />
      <RevealAuthorDialog post={revealing ? ref : undefined} onClose={() => setRevealing(false)} />
      <ContentHistoryDialog content={history ? ref : undefined} onClose={() => setHistory(false)} />
    </article>
  );
}
