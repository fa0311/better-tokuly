import {
  type InfiniteData,
  useInfiniteQuery,
  useMutation,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { errorReason, identity, social } from "@tokuly/api";
import { profileQuery } from "@tokuly/core/profile";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, Page } from "#/components/ui/page";
import { Tabs } from "#/components/ui/tabs";
import { profileClient } from "#/lib/api";
import { errorMessage } from "#/lib/errors";
import { FollowButton } from "./follow-button";
import { followClient, followKeys, refreshFollows } from "./follow-data";
import { RowsSkeleton, SearchInput, Tag, UserRow, useDebounced } from "./parts";

/** The four lists of someone's follows, each at its own address under `/u/<handle>/`. */
export type FollowListTab = "followers" | "following" | "mutuals" | "known-followers";

const LISTS: Record<FollowListTab, social.FollowList> = {
  followers: social.FollowList.FOLLOWERS,
  following: social.FollowList.FOLLOWING,
  mutuals: social.FollowList.MUTUALS,
  "known-followers": social.FollowList.KNOWN_FOLLOWERS,
};

const ROUTES = {
  followers: "/u/$handle/followers",
  following: "/u/$handle/following",
  mutuals: "/u/$handle/mutuals",
  "known-followers": "/u/$handle/known-followers",
} as const;

/** `/u/<handle>/followers` and its siblings: who follows them, whom they follow, and so on. */
export function FollowListsScreen({ handle, tab }: { handle: string; tab: FollowListTab }) {
  const { t } = useTranslation("follows");
  const navigate = useNavigate();
  // The same query as the profile screen, so coming from there costs nothing.
  const profile = useQuery(profileQuery(profileClient, handle));
  const user = profile.data?.profile?.user;
  const isSelf = profile.data?.profile?.relationship === identity.Relationship.SELF;
  // "Followed by people you follow" says nothing about your own account.
  const tabs = (Object.keys(LISTS) as FollowListTab[]).filter(
    (candidate) => !(isSelf && candidate === "known-followers"),
  );

  return (
    <Page title={user?.displayName ?? `@${handle}`} back={{ fallback: `/u/${handle}` }}>
      <Tabs<FollowListTab>
        className="mb-4"
        value={tab}
        onChange={(next) => navigate({ to: ROUTES[next], params: { handle }, replace: true })}
        items={tabs.map((value) => ({ value, label: t(`tabs.${value}`) }))}
      />
      {profile.isError ? (
        <EmptyState emoji="🫥" title={errorMessage(profile.error)} />
      ) : !user ? (
        <RowsSkeleton />
      ) : (
        <FollowList key={tab} userId={user.id} tab={tab} isSelf={isSelf} />
      )}
    </Page>
  );
}

function FollowList({
  userId,
  tab,
  isSelf,
}: {
  userId: string;
  tab: FollowListTab;
  isSelf: boolean;
}) {
  const { t } = useTranslation("follows");
  const queryClient = useQueryClient();
  const [filter, setFilter] = useState("");
  const query = useDebounced(filter);
  const key = followKeys.list(userId, LISTS[tab], query);
  const list = useInfiniteQuery({
    queryKey: key,
    queryFn: ({ pageParam }) =>
      followClient().listFollows({
        userId,
        list: LISTS[tab],
        query,
        page: { limit: 30, cursor: pageParam },
      }),
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
    retry: (count, error) => errorReason(error) !== "FOLLOW_LIST_PRIVATE" && count < 2,
  });
  const [removing, setRemoving] = useState<social.FollowEntry>();
  const [confirmOpen, setConfirmOpen] = useState(false);

  const remove = useMutation({
    mutationFn: (entry: social.FollowEntry) =>
      followClient().removeFollower({ userId: entry.user!.id }),
    onSuccess: (_, entry) => {
      queryClient.setQueryData<InfiniteData<social.ListFollowsResponse>>(key, (data) =>
        data
          ? {
              ...data,
              pages: data.pages.map((page) => ({
                ...page,
                total: Math.max(0, page.total - 1),
                entries: page.entries.filter((row) => row.user?.id !== entry.user?.id),
              })),
            }
          : data,
      );
      setConfirmOpen(false);
      toast.success(t("toast.removed"));
      return refreshFollows(queryClient);
    },
    onError: (error) => {
      setConfirmOpen(false);
      toast.error(errorMessage(error, { namespace: "follows" }));
    },
  });

  if (list.isError && errorReason(list.error) === "FOLLOW_LIST_PRIVATE")
    return <EmptyState emoji="🔒" title={t("private.title")} body={t("private.body")} />;

  const entries = list.data?.pages.flatMap((page) => page.entries) ?? [];
  const total = list.data?.pages[0]?.total ?? 0;
  const canRemove = isSelf && tab === "followers";

  return (
    <div className="flex flex-col gap-2">
      <SearchInput value={filter} onChange={setFilter} placeholder={t("list.filter")} />
      {list.isPending ? (
        <RowsSkeleton />
      ) : list.isError ? (
        <EmptyState emoji="😵" title={errorMessage(list.error, { namespace: "follows" })} />
      ) : entries.length === 0 ? (
        <EmptyState
          emoji={tab === "followers" ? "🌱" : "👀"}
          title={query ? t("list.noMatch") : t(`list.empty.${tab}`)}
        />
      ) : (
        <>
          <p className="px-3 pt-1 text-xs font-bold text-ink-faint">
            {t("list.count", { count: total })}
          </p>
          {entries.map(
            (entry) =>
              entry.user && (
                <UserRow
                  key={entry.user.id}
                  testId="follow-row"
                  user={entry.user}
                  subtitle={
                    <span className="flex items-center gap-2">
                      <span className="truncate">@{entry.user.handle}</span>
                      {entry.relation?.followsYou && !entry.relation.self && (
                        <Tag>{t("tag.followsYou")}</Tag>
                      )}
                      {entry.relation?.friends && <Tag>{t("tag.friend")}</Tag>}
                    </span>
                  }
                  trailing={
                    <span className="flex items-center gap-1.5">
                      <FollowButton user={entry.user} relation={entry.relation} />
                      {canRemove && (
                        <Button
                          size="sm"
                          variant="ghost"
                          onClick={() => {
                            setRemoving(entry);
                            setConfirmOpen(true);
                          }}
                        >
                          {t("list.remove")}
                        </Button>
                      )}
                    </span>
                  }
                />
              ),
          )}
          <LoadMore query={list} />
        </>
      )}
      <ConfirmDialog
        open={confirmOpen}
        onOpenChange={setConfirmOpen}
        title={t("removeFollower.title")}
        description={t("removeFollower.body", { name: removing?.user?.displayName ?? "" })}
        confirmLabel={t("removeFollower.confirm")}
        danger
        loading={remove.isPending}
        onConfirm={() => removing && remove.mutate(removing)}
      />
    </div>
  );
}
