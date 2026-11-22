import { useMutation, useQueryClient } from "@tanstack/react-query";
import type { engagement } from "@tokuly/api";
import { BookmarkX, Check, FolderPlus } from "lucide-react";
import { type FormEvent, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { create } from "zustand";
import { Button } from "#/components/ui/button";
import { Dialog } from "#/components/ui/dialog";
import { Skeleton } from "#/components/ui/page";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { bookmarkClient, engagementKeys, refreshBookmarks, useBookmark, useCollectionsFor, useEngagement } from "./api";
import type { Subject } from "./model";

/** The longest collection name the server takes. */
export const COLLECTION_NAME_MAX = 50;

const useChooser = create<{ subject: Subject | null }>(() => ({ subject: null }));

/** Opens the collection chooser for a subject, from anywhere (a button, a toast, a list row). */
export function openCollectionChooser(subject: Subject): void {
  useChooser.setState({ subject });
}

/** The one chooser of the app; mounted with the engagement background so it outlives the control that opened it. */
export function CollectionChooserHost() {
  const subject = useChooser((state) => state.subject);
  if (!subject) return null;
  return <CollectionChooser key={`${subject.type}:${subject.id}`} subject={subject} onClose={() => useChooser.setState({ subject: null })} />;
}

/** A collection's name as people see it: the default one is called "saved" by the server and gets a localized name. */
export function useCollectionName(): (collection: Pick<engagement.Collection, "name" | "isDefault">) => string {
  const { t } = useTranslation("engagement");
  return (collection) => (collection.isDefault ? t("bookmarks.defaultCollection") : collection.name);
}

/**
 * Which of the viewer's collections hold a subject: tick to add it, untick
 * to take it out, create a new collection on the spot, or stop saving it
 * altogether.
 */
function CollectionChooser({ subject, onClose }: { subject: Subject; onClose(): void }) {
  const { t } = useTranslation("engagement");
  const queryClient = useQueryClient();
  const collections = useCollectionsFor(subject, true);
  const { data: state } = useEngagement(subject);
  const bookmark = useBookmark(subject);
  const [name, setName] = useState("");
  const [busyId, setBusyId] = useState<string>();
  const reread = () => queryClient.invalidateQueries({ queryKey: engagementKeys.collectionsFor(subject) });

  const toggle = async (collection: engagement.Collection) => {
    setBusyId(collection.id);
    if (collection.containsSubject) await bookmark.remove(collection.id);
    else await bookmark.save(collection.id);
    await reread();
    setBusyId(undefined);
  };

  const createCollection = useMutation({
    mutationFn: async () => {
      const created = await bookmarkClient().createCollection({ name: name.trim() });
      if (created.collection) await bookmark.save(created.collection.id);
    },
    onSuccess: async () => {
      setName("");
      await Promise.all([reread(), refreshBookmarks(queryClient)]);
    },
    onError: (error) => toast.error(errorMessage(error)),
  });

  const stopSaving = async () => {
    if ((await bookmark.remove()) === false) {
      toast(t("bookmarks.removed"));
      onClose();
    }
  };

  const named = collections.data?.filter((collection) => !collection.isDefault) ?? [];
  const onSubmit = (event: FormEvent) => {
    event.preventDefault();
    if (name.trim()) createCollection.mutate();
  };

  return (
    <Dialog
      open
      onOpenChange={(open) => !open && onClose()}
      title={t("bookmarks.chooserTitle")}
      description={t("bookmarks.chooserLead")}
      footer={
        <>
          {state?.bookmarked && (
            <Button variant="ghost" className="mr-auto text-danger" icon={<BookmarkX className="size-4" />} onClick={stopSaving}>
              {t("bookmarks.remove")}
            </Button>
          )}
          <Button onClick={onClose}>{t("bookmarks.done")}</Button>
        </>
      }
    >
      {collections.isPending ? (
        <div className="flex flex-col gap-2">
          <Skeleton className="h-12" />
          <Skeleton className="h-12" />
        </div>
      ) : (
        <ul className="flex flex-col gap-1">
          {named.length === 0 && <li className="px-1 pb-2 text-sm text-ink-soft">{t("bookmarks.noCollections")}</li>}
          {named.map((collection) => (
            <li key={collection.id}>
              {/* biome-ignore lint/a11y/useSemanticElements: a row with a name and a count that is ticked as a whole, not a bare checkbox. */}
              <button
                type="button"
                role="checkbox"
                aria-checked={collection.containsSubject}
                disabled={busyId !== undefined}
                onClick={() => void toggle(collection)}
                className="flex w-full items-center gap-3 rounded-2xl px-3 py-2.5 text-left hover:bg-surface-2 disabled:opacity-60"
              >
                <span className={cn("grid size-6 shrink-0 place-items-center rounded-lg border-2", collection.containsSubject ? "border-ink bg-lime text-[#17122b]" : "border-line")}>
                  {collection.containsSubject && <Check className="size-4" strokeWidth={3.5} />}
                </span>
                <span className="min-w-0 flex-1 truncate font-bold">{collection.name}</span>
                <span className="text-sm font-bold text-ink-faint tabular-nums">{collection.itemCount}</span>
              </button>
            </li>
          ))}
        </ul>
      )}
      <form onSubmit={onSubmit} className="mt-3 flex items-center gap-2">
        <label className="flex h-11 min-w-0 flex-1 items-center gap-2 rounded-control border-2 border-line bg-surface px-3 focus-within:border-brand">
          <FolderPlus className="size-5 shrink-0 text-ink-faint" />
          <input
            value={name}
            onChange={(event) => setName(event.target.value)}
            maxLength={COLLECTION_NAME_MAX}
            placeholder={t("bookmarks.newCollection")}
            aria-label={t("bookmarks.newCollection")}
            className="min-w-0 flex-1 bg-transparent font-bold outline-none placeholder:font-medium placeholder:text-ink-faint"
          />
        </label>
        <Button type="submit" variant="secondary" size="sm" disabled={!name.trim()} loading={createCollection.isPending}>
          {t("bookmarks.create")}
        </Button>
      </form>
    </Dialog>
  );
}
