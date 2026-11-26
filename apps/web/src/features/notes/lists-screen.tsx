import { type InfiniteData, useMutation, useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import type { notes } from "@tokuly/api";
import { listKeys } from "@tokuly/core/notes";
import {
  Globe,
  ListPlus,
  Lock,
  LogOut,
  MoreHorizontal,
  Pencil,
  Plus,
  Trash2,
  UserPlus,
  X,
} from "lucide-react";
import { type ReactNode, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { Dialog } from "#/components/ui/dialog";
import { TextField } from "#/components/ui/field";
import { LoadMore } from "#/components/ui/load-more";
import { Menu } from "#/components/ui/menu";
import { EmptyState, ListRow, Skeleton } from "#/components/ui/page";
import { Tabs } from "#/components/ui/tabs";
import { errorMessage } from "#/lib/errors";
import { useMe } from "#/stores/session";
import {
  listClient,
  resetListTimeline,
  storeList,
  useFollowedLists,
  useList,
  useListMembers,
  useLists,
  useListsContainingMe,
  useUserSearch,
} from "./lists-api";
import { LISTS_NS, ListEditor } from "./lists-editor";
import { ComposeButton, NotesPage } from "./shell";
import { Timeline, type TimelineHandle } from "./timeline";

type ListsTab = "mine" | "followed" | "containing";

/** One list as a row: its name, whether it is public, how many people, and whose it is. */
export function ListCard({ list, showOwner }: { list: notes.NoteList; showOwner?: boolean }) {
  const { t } = useTranslation("notelists");
  return (
    <Link
      to="/notes/lists/$listId"
      params={{ listId: list.id }}
      data-testid="note-list"
      className="pressable sticker-sm flex items-center gap-3 rounded-card bg-surface p-3.5"
    >
      <span className="grid size-11 shrink-0 place-items-center rounded-2xl border-2 border-ink bg-sky text-[#17122b]">
        {list.public ? <Globe className="size-5" /> : <Lock className="size-5" />}
      </span>
      <span className="min-w-0 flex-1">
        <span className="block truncate font-black">{list.name}</span>
        <span className="block truncate text-sm text-ink-soft">
          {[
            list.public ? t("lists.public") : t("lists.private"),
            t("lists.members", { count: list.memberCount }),
            list.public ? t("lists.followers", { count: list.followerCount }) : null,
            showOwner && list.owner
              ? t("lists.by", { name: list.owner.displayName || `@${list.owner.handle}` })
              : null,
          ]
            .filter(Boolean)
            .join(" · ")}
        </span>
      </span>
    </Link>
  );
}

function RowsSkeleton() {
  return (
    <div className="flex flex-col gap-2" aria-busy>
      {[0, 1, 2].map((index) => (
        <Skeleton key={index} className="h-[4.5rem] rounded-card" />
      ))}
    </div>
  );
}

/** `/notes/lists`: the viewer's lists, the lists they follow, and the public lists they are on. */
export function ListsScreen() {
  const { t } = useTranslation("notelists");
  const [tab, setTab] = useState<ListsTab>("mine");
  const [creating, setCreating] = useState(false);
  return (
    <NotesPage
      title={t("lists.title")}
      back="/notes"
      scrollId="note-lists"
      actions={
        <Button size="sm" icon={<Plus className="size-4" />} onClick={() => setCreating(true)}>
          {t("lists.new")}
        </Button>
      }
      toolbar={
        <Tabs<ListsTab>
          value={tab}
          onChange={setTab}
          items={[
            { value: "mine", label: t("lists.tabs.mine") },
            { value: "followed", label: t("lists.tabs.followed") },
            { value: "containing", label: t("lists.tabs.containing") },
          ]}
        />
      }
    >
      <p className="mb-3 px-1 text-sm text-ink-soft">{t("lists.lead")}</p>
      {tab === "mine" && <MyLists onCreate={() => setCreating(true)} />}
      {tab === "followed" && <FollowedLists />}
      {tab === "containing" && <ListsWithMe />}
      <ListEditor open={creating} onOpenChange={setCreating} />
    </NotesPage>
  );
}

function MyLists({ onCreate }: { onCreate(): void }) {
  const { t } = useTranslation("notelists");
  const lists = useLists();
  if (lists.isPending) return <RowsSkeleton />;
  if (lists.isError) return <EmptyState emoji="😵" title={errorMessage(lists.error, LISTS_NS)} />;
  if (lists.data.length === 0)
    return (
      <EmptyState
        emoji="📋"
        title={t("lists.empty.mine")}
        body={t("lists.empty.mineBody")}
        action={
          <Button icon={<ListPlus className="size-4" />} onClick={onCreate}>
            {t("lists.create")}
          </Button>
        }
      />
    );
  return (
    <div className="flex flex-col gap-2">
      {lists.data.map((list) => (
        <ListCard key={list.id} list={list} />
      ))}
    </div>
  );
}

function PagedLists({
  query,
  empty,
}: {
  query: ReturnType<typeof useFollowedLists> | ReturnType<typeof useListsContainingMe>;
  empty: ReactNode;
}) {
  if (query.isPending) return <RowsSkeleton />;
  if (query.isError) return <EmptyState emoji="😵" title={errorMessage(query.error, LISTS_NS)} />;
  const lists = query.data.pages.flatMap((page) => page.lists);
  if (lists.length === 0) return empty;
  return (
    <div className="flex flex-col gap-2">
      {lists.map((list) => (
        <ListCard key={list.id} list={list} showOwner />
      ))}
      <LoadMore query={query} />
    </div>
  );
}

function FollowedLists() {
  const { t } = useTranslation("notelists");
  return (
    <PagedLists
      query={useFollowedLists()}
      empty={
        <EmptyState
          emoji="🔭"
          title={t("lists.empty.followed")}
          body={t("lists.empty.followedBody")}
        />
      }
    />
  );
}

function ListsWithMe() {
  const { t } = useTranslation("notelists");
  return (
    <PagedLists
      query={useListsContainingMe()}
      empty={
        <EmptyState
          emoji="🫶"
          title={t("lists.empty.containing")}
          body={t("lists.empty.containingBody")}
        />
      }
    />
  );
}

type ListTab = "timeline" | "members";

/** `/notes/lists/<id>`: a list's timeline and members; its owner manages it, others follow or leave it. */
export function ListScreen({ listId }: { listId: string }) {
  const { t } = useTranslation("notelists");
  const me = useMe()?.user?.id;
  const timeline = useRef<TimelineHandle>(null);
  const [tab, setTab] = useState<ListTab>("timeline");
  const query = useList(listId);
  const list = query.data;

  if (query.isError || query.data === null) {
    return (
      <NotesPage title={t("list.notFound")} back="/notes/lists" scrollId={`list-${listId}`}>
        <EmptyState
          emoji="🫥"
          title={query.isError ? errorMessage(query.error, LISTS_NS) : t("list.notFound")}
        />
      </NotesPage>
    );
  }
  const own = Boolean(list && me && list.owner?.id === me);
  return (
    <NotesPage
      title={list?.name ?? t("lists.title")}
      back="/notes/lists"
      scrollId={`list-${listId}`}
      onRefresh={() => timeline.current?.refresh() ?? Promise.resolve()}
      actions={list && <ListMenu list={list} own={own} />}
      toolbar={
        <Tabs<ListTab>
          value={tab}
          onChange={setTab}
          items={[
            { value: "timeline", label: t("list.tabs.timeline") },
            { value: "members", label: t("list.tabs.members") },
          ]}
        />
      }
    >
      {!list ? (
        <RowsSkeleton />
      ) : (
        <>
          <ListHeader list={list} own={own} />
          {tab === "timeline" ? (
            <Timeline
              ref={timeline}
              source={{ kind: "list", listId }}
              empty={
                <EmptyState
                  emoji="📭"
                  title={t("list.emptyTimeline")}
                  body={t("list.emptyTimelineBody")}
                />
              }
            />
          ) : (
            <Members list={list} own={own} />
          )}
        </>
      )}
      <ComposeButton />
    </NotesPage>
  );
}

function ListHeader({ list, own }: { list: notes.NoteList; own: boolean }) {
  const { t } = useTranslation("notelists");
  const queryClient = useQueryClient();
  const follow = useMutation({
    mutationFn: async () => {
      if (!list.following) return (await listClient().followList({ listId: list.id })).list;
      await listClient().unfollowList({ listId: list.id });
      return { ...list, following: false, followerCount: Math.max(0, list.followerCount - 1) };
    },
    onSuccess: async (next) => {
      storeList(queryClient, next);
      toast.success(next?.following ? t("toast.followed") : t("toast.unfollowed"));
      await queryClient.invalidateQueries({ queryKey: listKeys.followed });
    },
    onError: (error) => toast.error(errorMessage(error, LISTS_NS)),
  });
  return (
    <div className="sticker mb-3 flex flex-col gap-3 rounded-card bg-surface p-4">
      <div className="flex items-center gap-3">
        <span className="grid size-12 shrink-0 place-items-center rounded-2xl border-2 border-ink bg-sky text-[#17122b]">
          {list.public ? <Globe className="size-6" /> : <Lock className="size-6" />}
        </span>
        <div className="min-w-0 flex-1">
          <p className="truncate text-lg font-black">{list.name}</p>
          <p className="truncate text-sm font-bold text-ink-soft" data-testid="list-counts">
            {[
              list.public ? t("lists.public") : t("lists.private"),
              t("lists.members", { count: list.memberCount }),
              list.public ? t("lists.followers", { count: list.followerCount }) : null,
            ]
              .filter(Boolean)
              .join(" · ")}
          </p>
        </div>
        {!own && list.public && (
          <Button
            size="sm"
            variant={list.following ? "secondary" : "lime"}
            loading={follow.isPending}
            onClick={() => follow.mutate()}
          >
            {list.following ? t("list.following") : t("list.follow")}
          </Button>
        )}
      </div>
      {list.description && <p className="whitespace-pre-wrap text-sm">{list.description}</p>}
      {!own && list.owner && (
        <Link
          to="/u/$handle"
          params={{ handle: list.owner.handle }}
          className="flex items-center gap-2 text-sm font-bold text-ink-soft hover:text-ink"
        >
          <Avatar user={list.owner} size="xs" />
          {t("lists.by", { name: list.owner.displayName || `@${list.owner.handle}` })}
        </Link>
      )}
    </div>
  );
}

/** The header's "…" menu: edit and delete for the owner, leave for a member. */
function ListMenu({ list, own }: { list: notes.NoteList; own: boolean }) {
  const { t } = useTranslation("notelists");
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const [editing, setEditing] = useState(false);
  const [confirm, setConfirm] = useState<"delete" | "leave" | null>(null);
  const done = useMutation({
    mutationFn: async (what: "delete" | "leave") => {
      if (what === "delete") await listClient().deleteList({ listId: list.id });
      else await listClient().leaveList({ listId: list.id });
      return what;
    },
    onSuccess: async (what) => {
      setConfirm(null);
      toast.success(what === "delete" ? t("toast.listDeleted") : t("toast.left"));
      await queryClient.invalidateQueries({ queryKey: listKeys.all });
      if (what === "delete") {
        queryClient.removeQueries({ queryKey: listKeys.list(list.id) });
        void navigate({ to: "/notes/lists" });
      } else {
        await resetListTimeline(queryClient, list.id);
      }
    },
    onError: (error) => toast.error(errorMessage(error, LISTS_NS)),
  });
  const actions = own
    ? [
        {
          label: t("list.edit"),
          icon: <Pencil className="size-4" />,
          onSelect: () => setEditing(true),
        },
        {
          label: t("list.delete"),
          icon: <Trash2 className="size-4" />,
          danger: true,
          onSelect: () => setConfirm("delete"),
        },
      ]
    : list.isMember
      ? [
          {
            label: t("list.leave"),
            icon: <LogOut className="size-4" />,
            danger: true,
            onSelect: () => setConfirm("leave"),
          },
        ]
      : [];
  if (actions.length === 0) return null;
  const key = confirm === "delete" ? "list.deleteConfirm" : "list.leaveConfirm";
  return (
    <>
      <Menu
        trigger={
          <Button size="icon-sm" variant="ghost" aria-label={t("list.edit")}>
            <MoreHorizontal className="size-5" />
          </Button>
        }
        actions={actions}
      />
      {own && <ListEditor open={editing} onOpenChange={setEditing} list={list} />}
      <ConfirmDialog
        open={confirm !== null}
        onOpenChange={(open) => !open && setConfirm(null)}
        title={t(`${key}.title`, { name: list.name })}
        description={t(`${key}.body`)}
        confirmLabel={t(`${key}.confirm`)}
        danger
        loading={done.isPending}
        onConfirm={() => confirm && done.mutate(confirm)}
      />
    </>
  );
}

function Members({ list, own }: { list: notes.NoteList; own: boolean }) {
  const { t } = useTranslation("notelists");
  const queryClient = useQueryClient();
  const [adding, setAdding] = useState(false);
  const members = useListMembers(list.id);
  const remove = useMutation({
    mutationFn: (userId: string) =>
      listClient().removeListMembers({ listId: list.id, userIds: [userId] }),
    onSuccess: async (response, userId) => {
      queryClient.setQueryData<InfiniteData<notes.ListListMembersResponse>>(
        listKeys.members(list.id),
        (data) =>
          data && {
            ...data,
            pages: data.pages.map((page) => ({
              ...page,
              members: page.members.filter((member) => member.id !== userId),
            })),
          },
      );
      storeList(queryClient, response.list);
      toast.success(t("toast.removed"));
      await resetListTimeline(queryClient, list.id);
    },
    onError: (error) => toast.error(errorMessage(error, LISTS_NS)),
  });
  const rows = members.data?.pages.flatMap((page) => page.members) ?? [];
  return (
    <div className="flex flex-col gap-1">
      {own && (
        <Button
          variant="secondary"
          className="mb-2 self-start"
          icon={<UserPlus className="size-4" />}
          onClick={() => setAdding(true)}
        >
          {t("list.addMembers")}
        </Button>
      )}
      {members.isPending ? (
        <RowsSkeleton />
      ) : rows.length === 0 ? (
        <EmptyState
          emoji="🌱"
          title={t("list.noMembers")}
          body={own ? t("list.noMembersBody") : undefined}
        />
      ) : (
        <>
          {rows.map((member) => (
            <div key={member.id} data-testid="list-member" className="flex items-center">
              <Link
                to="/u/$handle"
                params={{ handle: member.handle }}
                className="min-w-0 flex-1 rounded-2xl hover:bg-surface-2"
              >
                <ListRow
                  leading={<Avatar user={member} />}
                  title={member.displayName || `@${member.handle}`}
                  subtitle={`@${member.handle}`}
                />
              </Link>
              {own && (
                <Button
                  size="icon-sm"
                  variant="ghost"
                  aria-label={t("list.removeMember", { name: member.displayName || member.handle })}
                  disabled={remove.isPending}
                  onClick={() => remove.mutate(member.id)}
                >
                  <X className="size-4" />
                </Button>
              )}
            </div>
          ))}
          <LoadMore query={members} />
        </>
      )}
      {own && <AddMembers list={list} open={adding} onOpenChange={setAdding} />}
    </div>
  );
}

function useDebounced(value: string, ms = 300): string {
  const [debounced, setDebounced] = useState(value);
  useEffect(() => {
    const timer = setTimeout(() => setDebounced(value.trim()), ms);
    return () => clearTimeout(timer);
  }, [value, ms]);
  return debounced;
}

/** The sheet for finding people by name and putting them on the list. */
function AddMembers({
  list,
  open,
  onOpenChange,
}: {
  list: notes.NoteList;
  open: boolean;
  onOpenChange(open: boolean): void;
}) {
  const { t } = useTranslation("notelists");
  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("add.title")}
      description={t("add.lead")}
    >
      {open && <UserSearchAdd list={list} />}
    </Dialog>
  );
}

function UserSearchAdd({ list }: { list: notes.NoteList }) {
  const { t } = useTranslation("notelists");
  const queryClient = useQueryClient();
  const [text, setText] = useState("");
  const query = useDebounced(text);
  const results = useUserSearch(query);
  const [added, setAdded] = useState<ReadonlySet<string>>(new Set());
  const add = useMutation({
    mutationFn: (userId: string) =>
      listClient().addListMembers({ listId: list.id, userIds: [userId] }),
    onSuccess: async (response, userId) => {
      setAdded((current) => new Set(current).add(userId));
      storeList(queryClient, response.list);
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: listKeys.members(list.id) }),
        resetListTimeline(queryClient, list.id),
      ]);
    },
    onError: (error) => toast.error(errorMessage(error, LISTS_NS)),
  });
  const rows = results.data ?? [];
  return (
    <div className="flex flex-col gap-2">
      <TextField
        aria-label={t("add.search")}
        placeholder={t("add.search")}
        value={text}
        onChange={(event) => setText(event.target.value)}
        autoFocus
      />
      {query && results.isSuccess && rows.length === 0 && (
        <EmptyState emoji="🔍" title={t("add.noResults")} />
      )}
      {rows.map(
        (result) =>
          result.user && (
            <ListRow
              key={result.user.id}
              leading={<Avatar user={result.user} />}
              title={result.user.displayName || `@${result.user.handle}`}
              subtitle={`@${result.user.handle}`}
              trailing={
                added.has(result.user.id) ? (
                  <span className="rounded-full bg-surface-2 px-3 py-1 text-xs font-bold text-ink-soft">
                    {t("add.added")}
                  </span>
                ) : (
                  <Button
                    size="sm"
                    icon={<Plus className="size-4" />}
                    disabled={add.isPending}
                    onClick={() => add.mutate(result.user!.id)}
                  >
                    {t("add.add")}
                  </Button>
                )
              }
            />
          ),
      )}
    </div>
  );
}
