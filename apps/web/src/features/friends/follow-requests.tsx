import {
  type InfiniteData,
  useInfiniteQuery,
  useMutation,
  useQueryClient,
} from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { social } from "@tokuly/api";
import { Check, Settings } from "lucide-react";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { LoadMore } from "#/components/ui/load-more";
import { Page } from "#/components/ui/page";
import { errorMessage } from "#/lib/errors";
import { useTranslation } from "#/lib/use-translation";
import { followClient, followKeys, refreshFollows } from "./follow-data";
import { RowsSkeleton, SectionTitle, UserRow } from "./parts";

const NS = { namespace: "follows" };

function useRequestList(direction: social.FollowRequestDirection) {
  return useInfiniteQuery({
    queryKey: followKeys.requestList(direction),
    queryFn: ({ pageParam }) =>
      followClient().listFollowRequests({ direction, page: { limit: 30, cursor: pageParam } }),
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
  });
}

/** Takes one request out of a loaded list at once (the lists re-read behind it). */
function dropRequest(
  data: InfiniteData<social.ListFollowRequestsResponse> | undefined,
  requestId: string,
) {
  return data
    ? {
        ...data,
        pages: data.pages.map((page) => ({
          ...page,
          requests: page.requests.filter((request) => request.id !== requestId),
        })),
      }
    : data;
}

/**
 * Follow requests: people asking to follow the viewer's locked account
 * (承認 / 削除 — declining is silent), and the requests the viewer sent.
 */
export function FollowRequests() {
  const { t } = useTranslation("follows");
  const queryClient = useQueryClient();
  const incoming = useRequestList(social.FollowRequestDirection.INCOMING);
  const outgoing = useRequestList(social.FollowRequestDirection.OUTGOING);

  const respond = useMutation({
    mutationFn: (input: { requestId: string; approve: boolean }) =>
      followClient().respondFollowRequest(input),
    onSuccess: (_, input) => {
      queryClient.setQueryData(
        followKeys.requestList(social.FollowRequestDirection.INCOMING),
        (data: InfiniteData<social.ListFollowRequestsResponse> | undefined) =>
          dropRequest(data, input.requestId),
      );
      toast.success(input.approve ? t("requests.approved") : t("requests.declined"));
      return refreshFollows(queryClient);
    },
    onError: (error) => toast.error(errorMessage(error, NS)),
  });
  const cancel = useMutation({
    mutationFn: (requestId: string) => followClient().cancelFollowRequest({ requestId }),
    onSuccess: (_, requestId) => {
      queryClient.setQueryData(
        followKeys.requestList(social.FollowRequestDirection.OUTGOING),
        (data: InfiniteData<social.ListFollowRequestsResponse> | undefined) =>
          dropRequest(data, requestId),
      );
      toast.success(t("toast.withdrawn"));
      return refreshFollows(queryClient);
    },
    onError: (error) => toast.error(errorMessage(error, NS)),
  });

  const incomingRows = incoming.data?.pages.flatMap((page) => page.requests) ?? [];
  const outgoingRows = outgoing.data?.pages.flatMap((page) => page.requests) ?? [];

  return (
    <div className="flex flex-col gap-6" data-testid="follow-requests">
      <section>
        <SectionTitle>{t("requests.incoming")}</SectionTitle>
        {incoming.isPending ? (
          <RowsSkeleton />
        ) : incomingRows.length ? (
          <>
            {incomingRows.map(
              (request) =>
                request.from && (
                  <UserRow
                    key={request.id}
                    testId="follow-request"
                    user={request.from}
                    subtitle={
                      request.knownFollowerCount > 0
                        ? t("requests.known", { count: request.knownFollowerCount })
                        : undefined
                    }
                    trailing={
                      <span className="flex gap-1.5">
                        <Button
                          size="sm"
                          variant="lime"
                          icon={<Check className="size-4" />}
                          disabled={respond.isPending}
                          onClick={() => respond.mutate({ requestId: request.id, approve: true })}
                        >
                          {t("requests.approve")}
                        </Button>
                        <Button
                          size="sm"
                          variant="ghost"
                          disabled={respond.isPending}
                          onClick={() => respond.mutate({ requestId: request.id, approve: false })}
                        >
                          {t("requests.decline")}
                        </Button>
                      </span>
                    }
                  />
                ),
            )}
            <LoadMore query={incoming} />
          </>
        ) : (
          <p className="px-3 py-4 text-sm text-ink-faint">{t("requests.emptyIncoming")}</p>
        )}
        <p className="px-3 pt-1 text-xs text-ink-faint">{t("requests.silent")}</p>
      </section>
      <section>
        <SectionTitle>{t("requests.outgoing")}</SectionTitle>
        {outgoing.isPending ? (
          <RowsSkeleton />
        ) : outgoingRows.length ? (
          <>
            {outgoingRows.map(
              (request) =>
                request.to && (
                  <UserRow
                    key={request.id}
                    testId="follow-request-sent"
                    user={request.to}
                    trailing={
                      <Button
                        size="sm"
                        variant="secondary"
                        disabled={cancel.isPending}
                        onClick={() => cancel.mutate(request.id)}
                      >
                        {t("requests.cancel")}
                      </Button>
                    }
                  />
                ),
            )}
            <LoadMore query={outgoing} />
          </>
        ) : (
          <p className="px-3 py-4 text-sm text-ink-faint">{t("requests.emptyOutgoing")}</p>
        )}
      </section>
    </div>
  );
}

/** `/friends/follow-requests`: where the follow-request notification leads. */
export function FollowRequestsScreen() {
  const { t } = useTranslation("follows");
  return (
    <Page
      title={t("requests.title")}
      back={{ fallback: "/friends" }}
      actions={
        <Link
          to="/settings/follows"
          aria-label={t("settings.title")}
          className="grid size-9 place-items-center rounded-full hover:bg-surface-2"
        >
          <Settings className="size-5" />
        </Link>
      }
    >
      <FollowRequests />
    </Page>
  );
}
