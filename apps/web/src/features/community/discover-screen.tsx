import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { community } from "@tokuly/api";
import { CATEGORIES, handleOfPath, inviteCodeOf, refreshMembership } from "@tokuly/core/community";
import { KeyRound, Plus, Search, X } from "lucide-react";
import { useEffect, useState } from "react";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { Dialog } from "#/components/ui/dialog";
import { TextField } from "#/components/ui/field";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { SelectField } from "#/components/ui/select";
import { Tabs } from "#/components/ui/tabs";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { useTranslation } from "#/lib/use-translation";
import {
  communityClient,
  NS,
  useDirectory,
  useFollowedThreads,
  useFriendsCommunities,
  useMyCommunities,
  useMyEvents,
  usePopularTags,
  useSchoolCommunities,
  useTrending,
} from "./api";
import type { DiscoverSearch, DiscoverSort } from "./discover-search";
import { EventCard } from "./events";
import { CommunityCard, CommunityRow, RoleChip, SectionTitle } from "./parts";
import { ThreadCard } from "./thread-card";

const C = community.CommunityCategory;
const D = community.DirectorySort;

const SORTS: Record<DiscoverSort, community.DirectorySort> = {
  members: D.MEMBERS,
  new: D.NEW,
  active: D.ACTIVE,
};

/** `/communities`: the person's own communities, and the directory to find more. */
export function DiscoverScreen({
  search,
  onSearchChange,
}: {
  search: DiscoverSearch;
  onSearchChange(next: DiscoverSearch): void;
}) {
  const { t } = useTranslation(NS);
  const [entering, setEntering] = useState(false);
  const tab = search.tab ?? "my";
  return (
    <Page
      title={t("title")}
      back="/me"
      wide
      actions={
        <>
          <Button
            variant="ghost"
            size="icon-sm"
            aria-label={t("discover.enterCode")}
            onClick={() => setEntering(true)}
          >
            <KeyRound className="size-5" />
          </Button>
          <Link to="/communities/new">
            <Button size="sm" icon={<Plus className="size-4" />}>
              {t("create.short")}
            </Button>
          </Link>
        </>
      }
    >
      <Tabs
        items={[
          { value: "my", label: t("discover.my") },
          { value: "discover", label: t("discover.find") },
        ]}
        value={tab}
        onChange={(next) => onSearchChange(next === "discover" ? { tab: "discover" } : {})}
      />
      <div className="mt-4">
        {tab === "my" ? (
          <MyCommunities onDiscover={() => onSearchChange({ tab: "discover" })} />
        ) : (
          <Directory search={search} onSearchChange={onSearchChange} />
        )}
      </div>
      <InviteCodeDialog open={entering} onOpenChange={setEntering} />
    </Page>
  );
}

/** Typing (or pasting) an invite code or link leads to the invite's page. */
function InviteCodeDialog({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange(open: boolean): void;
}) {
  const { t } = useTranslation(NS);
  const navigate = useNavigate();
  const [value, setValue] = useState("");
  const code = inviteCodeOf(value);
  const go = () => {
    if (!code) return;
    onOpenChange(false);
    void navigate({ to: "/join/$code", params: { code } });
  };
  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("discover.enterCode")}
      description={t("discover.enterCodeBody")}
      footer={
        <Button disabled={!code} onClick={go}>
          {t("discover.openInvite")}
        </Button>
      }
    >
      <form
        onSubmit={(event) => {
          event.preventDefault();
          go();
        }}
      >
        <TextField
          label={t("join.codeLabel")}
          value={value}
          autoCapitalize="off"
          autoComplete="off"
          onChange={(event) => setValue(event.target.value)}
        />
      </form>
    </Dialog>
  );
}

function MyCommunities({ onDiscover }: { onDiscover(): void }) {
  const { t } = useTranslation(NS);
  const queryClient = useQueryClient();
  const mine = useMyCommunities();
  const events = useMyEvents();
  const followed = useFollowedThreads();
  const cancel = useMutation({
    mutationFn: (communityId: string) => communityClient().cancelJoinRequest({ communityId }),
    onSuccess: () => {
      refreshMembership(queryClient);
      toast(t("join.cancelled"));
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: NS })),
  });
  if (mine.isPending) return <Skeleton className="h-48" />;
  const communities = mine.communities ?? [];
  const requested = mine.requested ?? [];
  if (communities.length + requested.length === 0)
    return (
      <EmptyState
        emoji="🏕️"
        title={t("discover.myEmpty")}
        body={t("discover.myEmptyBody")}
        action={<Button onClick={onDiscover}>{t("discover.find")}</Button>}
      />
    );
  const unread = followed.items?.filter((thread) => thread.viewer?.unread) ?? [];
  return (
    <div className="grid gap-x-6 lg:grid-cols-2">
      <div className="min-w-0">
        <div className="rounded-card bg-surface p-1.5">
          {communities.map((c) => (
            <CommunityRow
              key={c.id}
              community={c}
              trailing={c.viewer && <RoleChip role={c.viewer.role} />}
            />
          ))}
          <LoadMore query={mine} />
        </div>
        {requested.length > 0 && (
          <>
            <SectionTitle>{t("discover.requested")}</SectionTitle>
            <div className="rounded-card bg-surface p-1.5">
              {requested.map((c) => (
                <CommunityRow
                  key={c.id}
                  community={c}
                  subtitle={t("join.waiting")}
                  trailing={
                    <Button
                      variant="ghost"
                      size="sm"
                      disabled={cancel.isPending}
                      onClick={(event) => {
                        // The row is a link; the button is its own action.
                        event.preventDefault();
                        cancel.mutate(c.id);
                      }}
                    >
                      {t("join.cancel")}
                    </Button>
                  }
                />
              ))}
            </div>
          </>
        )}
      </div>
      <div className="min-w-0">
        {(events.items?.length ?? 0) > 0 && (
          <>
            <SectionTitle>{t("discover.myEvents")}</SectionTitle>
            <div className="flex flex-col gap-2">
              {events.items?.map((event) => (
                <EventCard
                  key={event.id}
                  event={event}
                  handle={handleOfPath(event.path)}
                  communityName={
                    communities.find((c) => c.id === event.communityId)?.name ?? undefined
                  }
                />
              ))}
              <LoadMore query={events} />
            </div>
          </>
        )}
        {unread.length > 0 && (
          <>
            <SectionTitle>{t("discover.unreadThreads")}</SectionTitle>
            <div className="flex flex-col gap-2">
              {unread.map((thread) => (
                <ThreadCard key={thread.id} thread={thread} handle={handleOfPath(thread.path)} />
              ))}
            </div>
          </>
        )}
      </div>
    </div>
  );
}

const GRID = "grid gap-3 sm:grid-cols-2 lg:grid-cols-3";

function Directory({
  search,
  onSearchChange,
}: {
  search: DiscoverSearch;
  onSearchChange(next: DiscoverSearch): void;
}) {
  const { t } = useTranslation(NS);
  const [text, setText] = useState(search.q ?? "");
  // The address follows the field a moment after typing stops.
  useEffect(() => {
    const next = text.trim();
    if (next === (search.q ?? "")) return;
    const timer = window.setTimeout(() => onSearchChange({ ...search, q: next || undefined }), 300);
    return () => window.clearTimeout(timer);
  }, [text, search, onSearchChange]);

  const category = CATEGORIES.find((each) => each.key === search.category)?.value ?? C.UNSPECIFIED;
  const filtered = Boolean(search.q || search.tag || search.category);
  const directory = useDirectory({
    category,
    tag: search.tag ?? "",
    query: search.q ?? "",
    sort: SORTS[search.sort ?? "members"],
  });
  const tags = usePopularTags(category);

  return (
    <div>
      <TextField
        aria-label={t("discover.search")}
        placeholder={t("discover.search")}
        type="search"
        value={text}
        maxLength={64}
        onChange={(event) => setText(event.target.value)}
        trailing={
          text ? (
            <button
              type="button"
              aria-label={t("discover.clear")}
              onClick={() => setText("")}
              className="mr-1 grid size-9 place-items-center rounded-full hover:bg-surface-2"
            >
              <X className="size-4" />
            </button>
          ) : (
            <Search className="mr-2.5 size-5 text-ink-faint" />
          )
        }
      />
      <div className="scrollbar-none -mx-4 mt-3 flex gap-2 overflow-x-auto px-4 pb-1">
        {CATEGORIES.map((each) => {
          const active = search.category === each.key;
          return (
            <button
              key={each.key}
              type="button"
              aria-pressed={active}
              onClick={() =>
                onSearchChange({
                  ...search,
                  category: active ? undefined : each.key,
                  tag: undefined,
                })
              }
              className={cn(
                "inline-flex h-10 shrink-0 items-center gap-1.5 rounded-full border-2 px-3.5 text-sm font-bold",
                active ? "border-ink bg-brand text-on-brand" : "border-line bg-surface",
              )}
            >
              <span aria-hidden>{each.emoji}</span>
              {t(`category.${each.key}`)}
            </button>
          );
        })}
      </div>
      {(tags.data?.length ?? 0) > 0 && (
        <div className="mt-2 flex flex-wrap gap-1.5">
          {tags.data?.map(({ tag }) => {
            const active = search.tag === tag;
            return (
              <button
                key={tag}
                type="button"
                aria-pressed={active}
                onClick={() => onSearchChange({ ...search, tag: active ? undefined : tag })}
                className={cn(
                  "inline-flex h-7 items-center rounded-full px-2.5 text-xs font-bold",
                  active
                    ? "bg-brand text-on-brand"
                    : "bg-surface-2 text-ink-soft hover:bg-surface-3",
                )}
              >
                #{tag}
              </button>
            );
          })}
        </div>
      )}

      {!filtered && <Shelves category={category} />}

      <SectionTitle
        action={
          <SelectField
            className="w-48"
            value={search.sort ?? "members"}
            onChange={(next) =>
              onSearchChange({ ...search, sort: next === "members" ? undefined : next })
            }
            options={(["members", "active", "new"] as const).map((value) => ({
              value,
              label: t(`discover.sort.${value}`),
            }))}
          />
        }
      >
        {filtered ? t("discover.results") : t("discover.all")}
      </SectionTitle>
      {directory.isPending ? (
        <div className={GRID}>
          {Array.from({ length: 6 }, (_, index) => (
            <Skeleton key={index} className="h-44" />
          ))}
        </div>
      ) : directory.error ? (
        <EmptyState emoji="🌧️" title={errorMessage(directory.error, { namespace: NS })} />
      ) : directory.items?.length === 0 ? (
        <EmptyState
          emoji="🔭"
          title={t("discover.none")}
          body={t("discover.noneBody")}
          action={
            <Link to="/communities/new">
              <Button>{t("create.short")}</Button>
            </Link>
          }
        />
      ) : (
        <>
          <div className={GRID}>
            {directory.items?.map((c) => (
              <CommunityCard key={c.id} community={c} />
            ))}
          </div>
          <LoadMore query={directory} />
        </>
      )}
    </div>
  );
}

/** What to look at before searching: busy communities, the ones friends are in, the ones of the school. */
function Shelves({ category }: { category: community.CommunityCategory }) {
  const { t } = useTranslation(NS);
  const trending = useTrending(category);
  const friends = useFriendsCommunities();
  const school = useSchoolCommunities();
  return (
    <>
      {(trending.data?.length ?? 0) > 0 && (
        <>
          <SectionTitle>🔥 {t("discover.trending")}</SectionTitle>
          <div className={GRID}>
            {trending.data?.slice(0, 6).map((c) => (
              <CommunityCard key={c.id} community={c} />
            ))}
          </div>
        </>
      )}
      {category === C.UNSPECIFIED && (friends.items?.length ?? 0) > 0 && (
        <>
          <SectionTitle>👯 {t("discover.friends")}</SectionTitle>
          <div className={GRID}>
            {friends.items?.map((entry) =>
              entry.community ? (
                <CommunityCard
                  key={entry.community.id}
                  community={entry.community}
                  footer={
                    <span className="mt-1 flex items-center gap-1.5 text-xs font-bold text-ink-soft">
                      <span className="flex -space-x-1.5">
                        {entry.friends.map((friend) => (
                          <Avatar key={friend.id} user={friend} size="xs" />
                        ))}
                      </span>
                      {t("discover.friendsIn", { count: entry.friendCount })}
                    </span>
                  }
                />
              ) : null,
            )}
          </div>
          <LoadMore query={friends} />
        </>
      )}
      {category === C.UNSPECIFIED && (school.items?.length ?? 0) > 0 && (
        <>
          <SectionTitle>🏫 {t("discover.school", { school: school.schoolName })}</SectionTitle>
          <div className={GRID}>
            {school.items?.map((c) => (
              <CommunityCard key={c.id} community={c} />
            ))}
          </div>
          <LoadMore query={school} />
        </>
      )}
    </>
  );
}
