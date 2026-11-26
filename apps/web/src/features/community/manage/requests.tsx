import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import type { community } from "@tokuly/api";
import { communityKeys } from "@tokuly/core/community";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { EmojiText } from "#/features/emoji";
import { errorMessage } from "#/lib/errors";
import { communityClient, NS, useJoinRequests } from "../api";
import { Age, Notice } from "../parts";

/** People asking to join, oldest first, with their answers. */
export function RequestsSection({ community: c }: { community: community.Community }) {
  const { t } = useTranslation(NS);
  const queryClient = useQueryClient();
  const requests = useJoinRequests(c.id);
  const respond = useMutation({
    mutationFn: (input: { requestId: string; approve: boolean }) =>
      communityClient().respondJoinRequest(input),
    onSuccess: (_, { approve }) => {
      void queryClient.invalidateQueries({ queryKey: communityKeys.requests(c.id) });
      void queryClient.invalidateQueries({ queryKey: communityKeys.memberLists(c.id) });
      void queryClient.invalidateQueries({ queryKey: communityKeys.communities });
      toast(approve ? t("manage.requests.approved") : t("manage.requests.declined"));
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: NS })),
  });
  if (requests.isPending) return <Skeleton className="h-32" />;
  if (requests.items?.length === 0)
    return <EmptyState emoji="📭" title={t("manage.requests.empty")} />;
  return (
    <div className="flex flex-col gap-3">
      <Notice>{t("manage.requests.hint")}</Notice>
      {requests.items?.map((request) => (
        <article key={request.id} className="rounded-card bg-surface p-4">
          <div className="flex items-center gap-3">
            <Avatar user={request.user} />
            <div className="min-w-0 flex-1">
              <Link
                to="/u/$handle"
                params={{ handle: request.user?.handle ?? "" }}
                className="block truncate font-bold"
              >
                <EmojiText text={request.user?.displayName || request.user?.handle || ""} />
              </Link>
              <p className="truncate text-sm text-ink-soft">
                @{request.user?.handle} ·{" "}
                <Age ms={Number(request.createdAt?.seconds ?? 0n) * 1000} />
              </p>
            </div>
          </div>
          {request.answers.length > 0 && (
            <dl className="mt-3 flex flex-col gap-2">
              {request.answers.map((answer, index) => (
                <div key={index} className="rounded-2xl bg-surface-2 px-3 py-2">
                  <dt className="text-xs font-bold text-ink-soft">{answer.question}</dt>
                  <dd className="whitespace-pre-wrap break-words text-sm">
                    <EmojiText text={answer.answer} />
                  </dd>
                </div>
              ))}
            </dl>
          )}
          <div className="mt-3 flex justify-end gap-2">
            <Button
              variant="ghost"
              size="sm"
              disabled={respond.isPending}
              onClick={() => respond.mutate({ requestId: request.id, approve: false })}
            >
              {t("manage.requests.decline")}
            </Button>
            <Button
              variant="lime"
              size="sm"
              disabled={respond.isPending}
              onClick={() => respond.mutate({ requestId: request.id, approve: true })}
            >
              {t("manage.requests.approve")}
            </Button>
          </div>
        </article>
      ))}
      <LoadMore query={requests} />
    </div>
  );
}
