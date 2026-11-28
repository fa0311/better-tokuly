import {
  type InfiniteData,
  useInfiniteQuery,
  useMutation,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import type { social } from "@tokuly/api";
import { BellOff, Check, Pencil, Plus, UserPlus, X } from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Dialog } from "#/components/ui/dialog";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, Page } from "#/components/ui/page";
import { errorMessage } from "#/lib/errors";
import { useTranslation } from "#/lib/use-translation";
import { CircleEditor } from "./circle-editor";
import { CircleBadge, useCircleName } from "./circles-screen";
import { circleClient, circleKeys } from "./follow-data";
import { RowsSkeleton, SearchInput, Tag, UserRow, useDebounced } from "./parts";

const NS = { namespace: "circles" };

/** Why someone can be in a circle: 友達 and/or フォロワー. */
function TieTags({ friend, follower }: { friend: boolean; follower: boolean }) {
  const { t } = useTranslation("circles");
  return (
    <>
      {friend && <Tag className="bg-sky text-[#17122b]">{t("tie.friend")}</Tag>}
      {follower && <Tag>{t("tie.follower")}</Tag>}
    </>
  );
}

/** `/circles/<id>`: who is in it, adding from friends and followers, removing. */
export function CircleScreen({ circleId }: { circleId: string }) {
  const { t } = useTranslation("circles");
  const queryClient = useQueryClient();
  const nameOf = useCircleName();
  const [editing, setEditing] = useState(false);
  const [adding, setAdding] = useState(false);
  const circle = useQuery({
    queryKey: circleKeys.circle(circleId),
    queryFn: async () => (await circleClient().getCircle({ circleId })).circle ?? null,
  });
  const members = useInfiniteQuery({
    queryKey: circleKeys.members(circleId),
    queryFn: ({ pageParam }) =>
      circleClient().listCircleMembers({ circleId, page: { limit: 50, cursor: pageParam } }),
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
  });

  const remove = useMutation({
    mutationFn: (userId: string) =>
      circleClient().removeCircleMembers({ circleId, userIds: [userId] }),
    onSuccess: (response, userId) => {
      queryClient.setQueryData<InfiniteData<social.ListCircleMembersResponse>>(
        circleKeys.members(circleId),
        (data) =>
          data && {
            ...data,
            pages: data.pages.map((page) => ({
              ...page,
              total: Math.max(0, page.total - 1),
              members: page.members.filter((member) => member.user?.id !== userId),
            })),
          },
      );
      if (response.circle) queryClient.setQueryData(circleKeys.circle(circleId), response.circle);
      toast.success(t("toast.removedMember"));
      return Promise.all([
        queryClient.invalidateQueries({ queryKey: circleKeys.list }),
        queryClient.invalidateQueries({ queryKey: ["social", "circles", "candidates", circleId] }),
      ]);
    },
    onError: (error) => toast.error(errorMessage(error, NS)),
  });

  const rows = members.data?.pages.flatMap((page) => page.members) ?? [];
  const current = circle.data;

  return (
    <Page
      title={current ? nameOf(current) : t("title")}
      back={{ fallback: "/circles" }}
      actions={
        current && (
          <Button
            size="icon-sm"
            variant="ghost"
            aria-label={t("edit")}
            onClick={() => setEditing(true)}
          >
            <Pencil className="size-4" />
          </Button>
        )
      }
    >
      {circle.isError ? (
        <EmptyState emoji="🫥" title={errorMessage(circle.error, NS)} />
      ) : !current ? (
        <RowsSkeleton />
      ) : (
        <>
          <div className="sticker flex items-center gap-3 rounded-card bg-surface p-4">
            <CircleBadge circle={current} className="size-14" />
            <div className="min-w-0 flex-1">
              <p className="truncate text-lg font-black">{nameOf(current)}</p>
              <p className="text-sm text-ink-soft" data-testid="circle-member-count">
                {t("members", { count: current.memberCount })}
              </p>
            </div>
            <Button icon={<UserPlus className="size-4" />} onClick={() => setAdding(true)}>
              {t("add")}
            </Button>
          </div>
          <p className="mt-3 flex items-center gap-2 px-1 text-sm font-bold text-ink-soft">
            <BellOff className="size-4 shrink-0" aria-hidden />
            {t("silent")}
          </p>

          <div className="mt-5 flex flex-col gap-1">
            {members.isPending ? (
              <RowsSkeleton />
            ) : rows.length === 0 ? (
              <EmptyState
                emoji="🌟"
                title={t("noMembers.title")}
                body={t("noMembers.body")}
                action={
                  <Button icon={<Plus className="size-4" />} onClick={() => setAdding(true)}>
                    {t("add")}
                  </Button>
                }
              />
            ) : (
              <>
                {rows.map(
                  (member) =>
                    member.user && (
                      <UserRow
                        key={member.user.id}
                        testId="circle-member"
                        user={member.user}
                        subtitle={
                          <span className="flex items-center gap-1.5">
                            <span className="truncate">@{member.user.handle}</span>
                            <TieTags friend={member.friend} follower={member.follower} />
                          </span>
                        }
                        trailing={
                          <Button
                            size="icon-sm"
                            variant="ghost"
                            aria-label={t("removeMember", { name: member.user.displayName })}
                            disabled={remove.isPending}
                            onClick={() => remove.mutate(member.user!.id)}
                          >
                            <X className="size-4" />
                          </Button>
                        }
                      />
                    ),
                )}
                <LoadMore query={members} />
              </>
            )}
          </div>
          <CircleEditor open={editing} onOpenChange={setEditing} circle={current} />
          <AddMembers circleId={circleId} open={adding} onOpenChange={setAdding} />
        </>
      )}
    </Page>
  );
}

/** The sheet of people who can be added: friends and followers, searchable. */
function AddMembers({
  circleId,
  open,
  onOpenChange,
}: {
  circleId: string;
  open: boolean;
  onOpenChange(open: boolean): void;
}) {
  const { t } = useTranslation("circles");
  return (
    <Dialog open={open} onOpenChange={onOpenChange} title={t("add")} description={t("addLead")}>
      {open && <Candidates circleId={circleId} />}
    </Dialog>
  );
}

function Candidates({ circleId }: { circleId: string }) {
  const { t } = useTranslation("circles");
  const queryClient = useQueryClient();
  const [filter, setFilter] = useState("");
  const query = useDebounced(filter);
  const key = circleKeys.candidates(circleId, query);
  const candidates = useInfiniteQuery({
    queryKey: key,
    queryFn: ({ pageParam }) =>
      circleClient().listCircleCandidates({
        circleId,
        query,
        page: { limit: 30, cursor: pageParam },
      }),
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
  });

  const add = useMutation({
    mutationFn: (userId: string) =>
      circleClient().addCircleMembers({ circleId, userIds: [userId] }),
    onSuccess: (response, userId) => {
      queryClient.setQueryData<InfiniteData<social.ListCircleCandidatesResponse>>(
        key,
        (data) =>
          data && {
            ...data,
            pages: data.pages.map((page) => ({
              ...page,
              candidates: page.candidates.map((candidate) =>
                candidate.user?.id === userId ? { ...candidate, member: true } : candidate,
              ),
            })),
          },
      );
      if (response.circle) queryClient.setQueryData(circleKeys.circle(circleId), response.circle);
      return Promise.all([
        queryClient.invalidateQueries({ queryKey: circleKeys.members(circleId) }),
        queryClient.invalidateQueries({ queryKey: circleKeys.list }),
      ]);
    },
    onError: (error) => toast.error(errorMessage(error, NS)),
  });

  const rows = candidates.data?.pages.flatMap((page) => page.candidates) ?? [];
  return (
    <div className="flex flex-col gap-2">
      <SearchInput value={filter} onChange={setFilter} placeholder={t("searchPlaceholder")} />
      {candidates.isPending ? (
        <RowsSkeleton />
      ) : candidates.isError ? (
        <EmptyState emoji="😵" title={errorMessage(candidates.error, NS)} />
      ) : rows.length === 0 ? (
        <EmptyState
          emoji="🔍"
          title={query ? t("noCandidates.match") : t("noCandidates.title")}
          body={query ? undefined : t("noCandidates.body")}
        />
      ) : (
        <>
          {rows.map(
            (candidate) =>
              candidate.user && (
                <UserRow
                  key={candidate.user.id}
                  testId="circle-candidate"
                  user={candidate.user}
                  subtitle={
                    <span className="flex items-center gap-1.5">
                      <TieTags friend={candidate.friend} follower={candidate.follower} />
                    </span>
                  }
                  trailing={
                    candidate.member ? (
                      <span className="inline-flex items-center gap-1 rounded-full bg-surface-2 px-3 py-1 text-xs font-bold text-ink-soft">
                        <Check className="size-3.5" aria-hidden />
                        {t("added")}
                      </span>
                    ) : (
                      <Button
                        size="sm"
                        icon={<Plus className="size-4" />}
                        disabled={add.isPending}
                        onClick={() => add.mutate(candidate.user!.id)}
                      >
                        {t("addOne")}
                      </Button>
                    )
                  }
                />
              ),
          )}
          <LoadMore query={candidates} />
        </>
      )}
    </div>
  );
}
