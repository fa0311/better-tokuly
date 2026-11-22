import { useMutation, useQueryClient } from "@tanstack/react-query";
import { engagement, toDate } from "@tokuly/api";
import { BookmarkX, Ellipsis, FolderOpen, FolderPlus, Pencil, Trash2, Undo2 } from "lucide-react";
import { type FormEvent, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { Dialog } from "#/components/ui/dialog";
import { TextField } from "#/components/ui/field";
import { LoadMore } from "#/components/ui/load-more";
import { Menu } from "#/components/ui/menu";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { Tabs } from "#/components/ui/tabs";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { bookmarkClient, engagementKeys, refreshBookmarks, useBookmark, useBookmarks, useCollections, useMyReactions, useReact } from "./api";
import { COLLECTION_NAME_MAX, openCollectionChooser, useCollectionName } from "./collection-chooser";
import type { Subject } from "./model";
import { SubjectCard } from "./subject-card";

export type BookmarksTab = "saved" | "reactions";

export interface BookmarksSearch {
  tab?: BookmarksTab;
  /** A collection's ID; absent = everything saved. */
  collection?: string;
}

/** Reads `/bookmarks?tab=…&collection=…`. */
export function parseBookmarksSearch(search: Record<string, unknown>): BookmarksSearch {
  return {
    tab: search.tab === "reactions" ? "reactions" : undefined,
    collection: typeof search.collection === "string" && search.collection ? search.collection : undefined,
  };
}

/**
 * Everything the viewer kept: what they saved, filed into collections they
 * can create, rename and delete, and what they reacted to.
 */
export function BookmarksScreen({ search, onSearchChange }: { search: BookmarksSearch; onSearchChange(next: BookmarksSearch): void }) {
  const { t } = useTranslation("engagement");
  const tab = search.tab ?? "saved";
  return (
    <Page title={t("bookmarks.title")} back="/me" wide>
      <Tabs<BookmarksTab>
        className="mb-4"
        value={tab}
        onChange={(next) => onSearchChange(next === "saved" ? {} : { tab: next })}
        items={[
          { value: "saved", label: t("bookmarks.tabs.saved") },
          { value: "reactions", label: t("bookmarks.tabs.reactions") },
        ]}
      />
      {tab === "saved" ? <Saved collectionId={search.collection ?? ""} onSelect={(id) => onSearchChange(id ? { collection: id } : {})} /> : <Reacted />}
    </Page>
  );
}

function Saved({ collectionId, onSelect }: { collectionId: string; onSelect(collectionId: string): void }) {
  const { t } = useTranslation("engagement");
  const collections = useCollections();
  const collectionName = useCollectionName();
  const [creating, setCreating] = useState(false);
  const selected = collections.data?.find((collection) => (collectionId ? collection.id === collectionId : collection.isDefault));

  return (
    <div className="grid gap-4 md:grid-cols-[15rem_minmax(0,1fr)] md:gap-6">
      <nav aria-label={t("bookmarks.collections")} className="scrollbar-none -mx-4 flex gap-2 overflow-x-auto px-4 pb-1 md:mx-0 md:flex-col md:gap-1 md:overflow-visible md:px-0">
        {collections.isPending && <Skeleton className="h-10 w-28 shrink-0 md:w-full" />}
        {collections.data?.map((collection) => {
          const active = selected?.id === collection.id;
          return (
            <button
              key={collection.id}
              type="button"
              aria-current={active ? "page" : undefined}
              onClick={() => onSelect(collection.isDefault ? "" : collection.id)}
              className={cn(
                "flex h-10 shrink-0 items-center gap-2 rounded-full px-4 text-sm font-black transition-colors md:h-11 md:rounded-2xl",
                active ? "sticker-sm bg-brand text-on-brand" : "bg-surface text-ink-soft hover:text-ink md:bg-transparent md:hover:bg-surface-2",
              )}
            >
              <span className="max-w-40 truncate md:max-w-none md:flex-1 md:text-left">{collectionName(collection)}</span>
              <span className={cn("tabular-nums", active ? "opacity-80" : "text-ink-faint")}>{collection.itemCount}</span>
            </button>
          );
        })}
        <button
          type="button"
          onClick={() => setCreating(true)}
          className="flex h-10 shrink-0 items-center gap-2 rounded-full border-2 border-dashed border-line px-4 text-sm font-black text-ink-soft hover:border-ink-faint hover:text-ink md:h-11 md:rounded-2xl"
        >
          <FolderPlus className="size-4" />
          {t("bookmarks.newCollection")}
        </button>
      </nav>

      <section className="min-w-0 max-w-2xl">
        {selected && !selected.isDefault && <CollectionHeader collection={selected} onDeleted={() => onSelect("")} />}
        {/* A collection ID from an old link that no longer exists falls back to an explanation, not an error. */}
        {collections.data && !selected ? (
          <EmptyState emoji="🗂️" title={t("bookmarks.collectionGone")} action={<Button onClick={() => onSelect("")}>{t("bookmarks.showAll")}</Button>} />
        ) : (
          <BookmarkList key={collectionId} collectionId={collectionId} />
        )}
      </section>

      {creating && <CollectionNameDialog onClose={() => setCreating(false)} onSaved={(collection) => onSelect(collection.id)} />}
    </div>
  );
}

function CollectionHeader({ collection, onDeleted }: { collection: engagement.Collection; onDeleted(): void }) {
  const { t } = useTranslation("engagement");
  const queryClient = useQueryClient();
  const [renaming, setRenaming] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const remove = useMutation({
    mutationFn: () => bookmarkClient().deleteCollection({ collectionId: collection.id }),
    onSuccess: async () => {
      setDeleting(false);
      onDeleted();
      await refreshBookmarks(queryClient);
      toast(t("bookmarks.collectionDeleted"));
    },
    onError: (error) => toast.error(errorMessage(error)),
  });

  return (
    <div className="mb-3 flex items-center gap-2">
      <FolderOpen className="size-5 shrink-0 text-brand" />
      <h2 className="min-w-0 flex-1 truncate text-lg font-black">{collection.name}</h2>
      <Menu
        trigger={
          <Button variant="ghost" size="icon-sm" aria-label={t("bookmarks.collectionMenu")}>
            <Ellipsis className="size-5" />
          </Button>
        }
        actions={[
          { label: t("bookmarks.rename"), icon: <Pencil />, onSelect: () => setRenaming(true) },
          { label: t("bookmarks.deleteCollection"), icon: <Trash2 />, danger: true, onSelect: () => setDeleting(true) },
        ]}
      />
      {renaming && <CollectionNameDialog collection={collection} onClose={() => setRenaming(false)} />}
      <ConfirmDialog
        open={deleting}
        onOpenChange={setDeleting}
        title={t("bookmarks.deleteCollectionTitle", { name: collection.name })}
        description={t("bookmarks.deleteCollectionBody")}
        confirmLabel={t("bookmarks.deleteCollection")}
        danger
        loading={remove.isPending}
        onConfirm={() => remove.mutate()}
      />
    </div>
  );
}

/** Names a new collection, or renames `collection`. */
function CollectionNameDialog({ collection, onClose, onSaved }: { collection?: engagement.Collection; onClose(): void; onSaved?(collection: engagement.Collection): void }) {
  const { t } = useTranslation("engagement");
  const queryClient = useQueryClient();
  const [name, setName] = useState(collection?.name ?? "");
  const save = useMutation({
    mutationFn: async () => {
      const trimmed = name.trim();
      const response = collection ? await bookmarkClient().updateCollection({ collectionId: collection.id, name: trimmed }) : await bookmarkClient().createCollection({ name: trimmed });
      return response.collection;
    },
    onSuccess: async (saved) => {
      await queryClient.invalidateQueries({ queryKey: engagementKeys.collections });
      toast.success(collection ? t("bookmarks.renamed") : t("bookmarks.created"));
      if (saved) onSaved?.(saved);
      onClose();
    },
    onError: (error) => toast.error(errorMessage(error)),
  });
  const onSubmit = (event: FormEvent) => {
    event.preventDefault();
    if (name.trim()) save.mutate();
  };

  return (
    <Dialog
      open
      onOpenChange={(open) => !open && onClose()}
      title={collection ? t("bookmarks.renameTitle") : t("bookmarks.createTitle")}
      footer={
        <>
          <Button variant="ghost" onClick={onClose} disabled={save.isPending}>
            {t("bookmarks.cancel")}
          </Button>
          <Button type="submit" form="collection-name" disabled={!name.trim() || name.trim() === collection?.name} loading={save.isPending}>
            {collection ? t("bookmarks.saveName") : t("bookmarks.create")}
          </Button>
        </>
      }
    >
      <form id="collection-name" onSubmit={onSubmit}>
        <TextField
          label={t("bookmarks.collectionName")}
          value={name}
          onChange={(event) => setName(event.target.value)}
          maxLength={COLLECTION_NAME_MAX}
          placeholder={t("bookmarks.collectionNamePlaceholder")}
          autoFocus
        />
      </form>
    </Dialog>
  );
}

function ListSkeleton() {
  return (
    <div className="flex flex-col gap-3">
      {[0, 1, 2].map((index) => (
        <Skeleton key={index} className="h-28" />
      ))}
    </div>
  );
}

function BookmarkList({ collectionId }: { collectionId: string }) {
  const { t } = useTranslation("engagement");
  const query = useBookmarks(collectionId);
  const bookmarks = query.data?.pages.flatMap((page) => page.bookmarks) ?? [];

  if (query.isPending) return <ListSkeleton />;
  if (query.isError) return <EmptyState emoji="🌧️" title={errorMessage(query.error)} action={<Button onClick={() => void query.refetch()}>{t("bookmarks.retry")}</Button>} />;
  if (bookmarks.length === 0) {
    return collectionId ? <EmptyState emoji="🗂️" title={t("bookmarks.emptyCollection")} body={t("bookmarks.emptyCollectionBody")} /> : <EmptyState emoji="🔖" title={t("bookmarks.empty")} body={t("bookmarks.emptyBody")} />;
  }
  return (
    <ul className="flex flex-col gap-4">
      {bookmarks.map((bookmark) => bookmark.subject?.subject && <BookmarkRow key={`${bookmark.subject.subject.type}:${bookmark.subject.subject.id}`} bookmark={bookmark} collectionId={collectionId} />)}
      <LoadMore query={query} showEnd />
    </ul>
  );
}

function BookmarkRow({ bookmark, collectionId }: { bookmark: engagement.Bookmark; collectionId: string }) {
  const { t, i18n } = useTranslation("engagement");
  const preview = bookmark.subject!;
  const subject: Subject = { type: preview.subject!.type, id: preview.subject!.id };
  const actions = useBookmark(subject);
  const savedAt = toDate(bookmark.savedAt);
  const remove = async () => {
    if ((await actions.remove(collectionId)) !== undefined) toast(collectionId ? t("bookmarks.removedFromCollection") : t("bookmarks.removed"));
  };

  return (
    <li data-testid="bookmark">
      <SubjectCard preview={preview} />
      <div className="mt-1.5 flex items-center gap-1 px-1 text-xs font-bold whitespace-nowrap text-ink-faint">
        {savedAt && <span className="truncate">{t("bookmarks.savedOn", { date: savedAt.toLocaleDateString(i18n.language, { month: "short", day: "numeric" }) })}</span>}
        <span className="flex-1" />
        {preview.available && (
          <button type="button" onClick={() => openCollectionChooser(subject)} className="inline-flex h-8 items-center gap-1 rounded-full px-2.5 hover:bg-surface-2 hover:text-ink">
            <FolderPlus className="size-4" />
            {t("bookmarks.file")}
          </button>
        )}
        <button type="button" onClick={() => void remove()} className="inline-flex h-8 items-center gap-1 rounded-full px-2.5 hover:bg-surface-2 hover:text-danger">
          <BookmarkX className="size-4" />
          {collectionId ? t("bookmarks.takeOut") : t("bookmarks.unsave")}
        </button>
      </div>
    </li>
  );
}

function Reacted() {
  const { t } = useTranslation("engagement");
  const query = useMyReactions();
  const subjects = query.data?.pages.flatMap((page) => page.subjects) ?? [];

  if (query.isPending) return <ListSkeleton />;
  if (subjects.length === 0) return <EmptyState emoji="💜" title={t("bookmarks.noReactions")} body={t("bookmarks.noReactionsBody")} />;
  return (
    <ul className="mx-auto flex max-w-2xl flex-col gap-4">
      {subjects.map((entry) => entry.subject?.subject && <ReactedRow key={`${entry.subject.subject.type}:${entry.subject.subject.id}`} entry={entry} />)}
      <LoadMore query={query} showEnd />
    </ul>
  );
}

function ReactedRow({ entry }: { entry: engagement.ReactedSubject }) {
  const { t, i18n } = useTranslation("engagement");
  const queryClient = useQueryClient();
  const preview = entry.subject!;
  const subject: Subject = { type: preview.subject!.type, id: preview.subject!.id };
  const react = useReact(subject);
  const reactedAt = toDate(entry.reactedAt);
  const glyph = entry.reaction?.kind === engagement.ReactionKind.EMOJI ? entry.reaction.emoji : entry.reaction?.kind === engagement.ReactionKind.DISLIKE ? "👎" : "💜";
  const takeBack = async () => {
    await react(undefined);
    // The server forgets a reaction by UUID even when the subject is gone; the list is the only place to see that.
    await queryClient.invalidateQueries({ queryKey: engagementKeys.myReactions });
  };

  return (
    <li data-testid="reacted">
      <SubjectCard preview={preview} />
      <div className="mt-1.5 flex items-center gap-2 px-1 text-xs font-bold text-ink-faint">
        <span className="text-base leading-none" role="img" aria-label={t("reactions.yours")}>
          {glyph}
        </span>
        {reactedAt && <span>{reactedAt.toLocaleDateString(i18n.language, { month: "short", day: "numeric" })}</span>}
        <span className="flex-1" />
        <button type="button" onClick={() => void takeBack()} className="inline-flex h-8 items-center gap-1 rounded-full px-2.5 hover:bg-surface-2 hover:text-danger">
          <Undo2 className="size-4" />
          {t("reactions.takeBack")}
        </button>
      </div>
    </li>
  );
}
