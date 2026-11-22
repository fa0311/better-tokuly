import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type { common, notes } from "@tokuly/api";
import { Check, GripVertical, Pencil, Plus, Settings2, Trash2 } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { Dialog } from "#/components/ui/dialog";
import { TextField } from "#/components/ui/field";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { Sortable } from "#/components/ui/sortable";
import { charCount, imageUrl } from "#/features/notes";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { useMe } from "#/stores/session";
import { storyClient, storyCover, storyKeys, useArchivedStories, useHighlights } from "./api";
import { openStoryViewer } from "./viewer-store";

const TITLE_MAX = 30;
/** A highlight holds at most this many stories. */
const MAX_STORIES = 100;

function refreshHighlights(queryClient: ReturnType<typeof useQueryClient>): Promise<unknown> {
  return Promise.all([
    queryClient.invalidateQueries({ queryKey: ["stories", "highlights"] }),
    queryClient.invalidateQueries({ queryKey: ["stories", "highlight"] }),
    // Stories say whether they are in a highlight.
    queryClient.invalidateQueries({ queryKey: storyKeys.archives }),
  ]);
}

/** Removes a highlight from the profile (its stories stay in the archive). Resolves to whether it worked. */
export function useDeleteHighlight(): (highlight: notes.Highlight) => Promise<boolean> {
  const { t } = useTranslation("stories");
  const queryClient = useQueryClient();
  return async (highlight) => {
    try {
      await storyClient().deleteHighlight({ highlightId: highlight.id });
      await refreshHighlights(queryClient);
      toast(t("highlights.deleted"));
      return true;
    } catch (error) {
      toast.error(errorMessage(error));
      return false;
    }
  };
}

/** A highlight's circle: its cover, its title. */
function HighlightCircle({ highlight, onClick }: { highlight: notes.Highlight; onClick(): void }) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="flex w-[4.75rem] shrink-0 flex-col items-center gap-1"
      data-testid="highlight"
    >
      <span className="pressable sticker-sm block size-16 overflow-hidden rounded-full bg-surface-3">
        {highlight.cover?.url && (
          <img
            src={imageUrl(highlight.cover, 64)}
            alt=""
            loading="lazy"
            className="size-full object-cover"
          />
        )}
      </span>
      <span className="w-full truncate text-center text-xs font-bold">{highlight.title}</span>
    </button>
  );
}

/**
 * The highlights on a profile: titled groups of stories that stay. A tap
 * plays one; on their own profile the viewer can add, edit, reorder and
 * delete them. Renders nothing on a profile without highlights.
 */
export function ProfileHighlights({ user, isSelf }: { user: common.UserSummary; isSelf: boolean }) {
  const { t } = useTranslation("stories");
  const highlights = useHighlights(user.id);
  const [creating, setCreating] = useState(false);
  const [managing, setManaging] = useState(false);
  const [opening, setOpening] = useState<string>();
  const list = highlights.data ?? [];

  const play = async (highlight: notes.Highlight) => {
    setOpening(highlight.id);
    try {
      const response = await storyClient().getHighlight({ highlightId: highlight.id });
      openStoryViewer(
        [{ user, stories: response.stories, highlight: response.highlight ?? highlight }],
        { storyId: response.stories[0]?.id },
      );
    } catch (error) {
      toast.error(errorMessage(error));
    } finally {
      setOpening(undefined);
    }
  };

  if (!isSelf && list.length === 0) return null;
  return (
    <section aria-label={t("highlights.title")} className="mt-5" data-testid="highlights">
      <ul className="scrollbar-none -mx-4 flex gap-2.5 overflow-x-auto px-4 pb-1">
        {isSelf && (
          <li className="shrink-0">
            <button
              type="button"
              onClick={() => setCreating(true)}
              className="flex w-[4.75rem] flex-col items-center gap-1"
            >
              <span className="grid size-16 place-items-center rounded-full border-2 border-dashed border-line text-ink-soft hover:border-ink-faint hover:text-ink">
                <Plus className="size-6" />
              </span>
              <span className="w-full truncate text-center text-xs font-bold text-ink-soft">
                {t("highlights.new")}
              </span>
            </button>
          </li>
        )}
        {highlights.isPending && <Skeleton className="size-16 shrink-0 rounded-full" />}
        {list.map((highlight) => (
          <li
            key={highlight.id}
            className={cn("shrink-0", opening === highlight.id && "animate-pulse")}
          >
            <HighlightCircle highlight={highlight} onClick={() => void play(highlight)} />
          </li>
        ))}
        {isSelf && list.length > 0 && (
          <li className="shrink-0">
            <button
              type="button"
              onClick={() => setManaging(true)}
              className="flex w-[4.75rem] flex-col items-center gap-1"
            >
              <span className="grid size-16 place-items-center rounded-full bg-surface-2 text-ink-soft hover:text-ink">
                <Settings2 className="size-6" />
              </span>
              <span className="w-full truncate text-center text-xs font-bold text-ink-soft">
                {t("highlights.manage")}
              </span>
            </button>
          </li>
        )}
      </ul>
      {creating && <HighlightEditor onClose={() => setCreating(false)} />}
      {managing && <ManageHighlights userId={user.id} onClose={() => setManaging(false)} />}
    </section>
  );
}

/** Creates a highlight, or changes the title and stories of one. */
export function HighlightEditor({
  highlight,
  initialStoryIds,
  onClose,
  onSaved,
}: {
  highlight?: notes.Highlight /** Stories ticked from the start (a new highlight made from one story). */;
  initialStoryIds?: string[];
  onClose(): void;
  onSaved?(): void;
}) {
  const { t, i18n } = useTranslation("stories");
  const queryClient = useQueryClient();
  const archive = useArchivedStories("");
  const [title, setTitle] = useState(highlight?.title ?? "");
  /** The stories of the highlight, in playing order. `undefined` until an existing highlight's stories arrived. */
  const [picked, setPicked] = useState<string[] | undefined>(
    highlight ? undefined : (initialStoryIds ?? []),
  );
  const current = useQuery({
    queryKey: storyKeys.highlight(highlight?.id ?? ""),
    queryFn: () => storyClient().getHighlight({ highlightId: highlight!.id }),
    enabled: Boolean(highlight),
  });
  if (picked === undefined && current.data)
    setPicked(current.data.stories.map((story) => story.id));
  const selected = picked ?? [];
  const stories = archive.data?.pages.flatMap((page) => page.stories) ?? [];

  const save = useMutation({
    mutationFn: async () => {
      const request = { title: title.trim(), storyIds: selected };
      if (highlight) await storyClient().updateHighlight({ highlightId: highlight.id, ...request });
      else await storyClient().createHighlight(request);
    },
    onSuccess: async () => {
      await refreshHighlights(queryClient);
      toast.success(highlight ? t("highlights.saved") : t("highlights.created"));
      onSaved?.();
      onClose();
    },
    onError: (error) => toast.error(errorMessage(error)),
  });

  const toggle = (storyId: string) => {
    if (selected.includes(storyId)) setPicked(selected.filter((id) => id !== storyId));
    else if (selected.length < MAX_STORIES) setPicked([...selected, storyId]);
  };
  const titleLength = charCount(title.trim());
  const valid = titleLength >= 1 && titleLength <= TITLE_MAX && selected.length > 0;

  return (
    <Dialog
      open
      onOpenChange={(open) => !open && !save.isPending && onClose()}
      title={highlight ? t("highlights.editTitle") : t("highlights.createTitle")}
      description={t("highlights.lead")}
      footer={
        <>
          <Button variant="ghost" onClick={onClose} disabled={save.isPending}>
            {t("highlights.cancel")}
          </Button>
          <Button disabled={!valid} loading={save.isPending} onClick={() => save.mutate()}>
            {highlight ? t("highlights.save") : t("highlights.create")}
          </Button>
        </>
      }
    >
      <TextField
        label={t("highlights.name")}
        value={title}
        maxLength={TITLE_MAX * 2}
        placeholder={t("highlights.namePlaceholder")}
        onChange={(event) => setTitle(event.target.value)}
        error={
          titleLength > TITLE_MAX ? t("highlights.nameTooLong", { count: TITLE_MAX }) : undefined
        }
      />
      <p className="mt-4 mb-1.5 px-1 text-sm font-bold text-ink-soft">
        {t("highlights.pick", { count: selected.length })}
      </p>
      {archive.isPending || (highlight && picked === undefined) ? (
        <div className="grid grid-cols-3 gap-2">
          <Skeleton className="aspect-[3/4]" />
          <Skeleton className="aspect-[3/4]" />
          <Skeleton className="aspect-[3/4]" />
        </div>
      ) : stories.length === 0 ? (
        <EmptyState
          emoji="📷"
          title={t("highlights.noStories")}
          body={t("highlights.noStoriesBody")}
        />
      ) : (
        <>
          <ul className="grid grid-cols-3 gap-2" data-testid="highlight-stories">
            {stories.map((story) => {
              const order = selected.indexOf(story.id);
              return (
                <li key={story.id}>
                  {/* biome-ignore lint/a11y/useSemanticElements: a picture tile that is ticked as a whole and shows its place in the order, not a bare checkbox. */}
                  <button
                    type="button"
                    role="checkbox"
                    aria-checked={order >= 0}
                    aria-label={
                      story.title ||
                      new Date(Number(story.createdAt?.seconds ?? 0n) * 1000).toLocaleDateString(
                        i18n.language,
                      )
                    }
                    onClick={() => toggle(story.id)}
                    className={cn(
                      "relative block aspect-[3/4] w-full overflow-hidden rounded-2xl border-2 bg-surface-3",
                      order >= 0 ? "border-ink shadow-pop-sm" : "border-line",
                    )}
                  >
                    <img
                      src={imageUrl(storyCover(story), 160)}
                      alt=""
                      loading="lazy"
                      className={cn(
                        "size-full object-cover transition-opacity",
                        order < 0 && "opacity-70",
                      )}
                    />
                    <span
                      className={cn(
                        "absolute top-1.5 right-1.5 grid size-6 place-items-center rounded-full border-2 border-ink text-xs font-black",
                        order >= 0 ? "bg-lime text-[#17122b]" : "bg-surface/80",
                      )}
                    >
                      {order >= 0 ? order + 1 : ""}
                    </span>
                    {story.title && (
                      <span className="absolute inset-x-0 bottom-0 truncate bg-black/55 px-1.5 py-0.5 text-left text-[11px] font-bold text-white">
                        {story.title}
                      </span>
                    )}
                  </button>
                </li>
              );
            })}
          </ul>
          <LoadMore query={archive} />
        </>
      )}
    </Dialog>
  );
}

/** Adds one story to a highlight: an existing one, or a new one made on the spot. */
export function HighlightPicker({ storyId, onClose }: { storyId: string; onClose(): void }) {
  const { t } = useTranslation("stories");
  const me = useMe();
  const queryClient = useQueryClient();
  const highlights = useHighlights(me?.user?.id ?? "");
  const [creating, setCreating] = useState(false);

  const add = useMutation({
    mutationFn: async (highlight: notes.Highlight) => {
      const current = await storyClient().getHighlight({ highlightId: highlight.id });
      const storyIds = current.stories.map((story) => story.id);
      if (storyIds.includes(storyId)) return false;
      await storyClient().updateHighlight({
        highlightId: highlight.id,
        title: highlight.title,
        storyIds: [...storyIds, storyId],
      });
      return true;
    },
    onSuccess: async (added) => {
      await refreshHighlights(queryClient);
      toast.success(added ? t("highlights.added") : t("highlights.alreadyIn"));
      onClose();
    },
    onError: (error) => toast.error(errorMessage(error)),
  });

  if (creating) return <HighlightEditor initialStoryIds={[storyId]} onClose={onClose} />;
  return (
    <Dialog open onOpenChange={(open) => !open && onClose()} title={t("highlights.addTitle")}>
      <ul className="flex flex-col gap-1">
        <li>
          <button
            type="button"
            onClick={() => setCreating(true)}
            className="flex w-full items-center gap-3 rounded-2xl px-2 py-2 text-left font-black hover:bg-surface-2"
          >
            <span className="grid size-12 place-items-center rounded-full border-2 border-dashed border-line">
              <Plus className="size-5" />
            </span>
            {t("highlights.new")}
          </button>
        </li>
        {highlights.isPending && <Skeleton className="h-14" />}
        {highlights.data?.map((highlight) => (
          <li key={highlight.id}>
            <button
              type="button"
              disabled={add.isPending}
              onClick={() => add.mutate(highlight)}
              className="flex w-full items-center gap-3 rounded-2xl px-2 py-2 text-left hover:bg-surface-2 disabled:opacity-60"
            >
              <span className="block size-12 shrink-0 overflow-hidden rounded-full border-2 border-ink bg-surface-3">
                {highlight.cover?.url && (
                  <img
                    src={imageUrl(highlight.cover, 48)}
                    alt=""
                    className="size-full object-cover"
                  />
                )}
              </span>
              <span className="min-w-0 flex-1">
                <span className="block truncate font-black">{highlight.title}</span>
                <span className="block text-sm text-ink-soft">
                  {t("highlights.count", { count: highlight.storyCount })}
                </span>
              </span>
              <Check className="size-5 text-ink-faint" />
            </button>
          </li>
        ))}
      </ul>
    </Dialog>
  );
}

/** The viewer's highlights as a list to drag into a new order, each with edit and delete. */
function ManageHighlights({ userId, onClose }: { userId: string; onClose(): void }) {
  const { t } = useTranslation("stories");
  const queryClient = useQueryClient();
  const highlights = useHighlights(userId);
  const deleteHighlight = useDeleteHighlight();
  const [editing, setEditing] = useState<notes.Highlight>();
  const [deleting, setDeleting] = useState<notes.Highlight>();

  const reorder = useMutation({
    mutationFn: (next: notes.Highlight[]) =>
      storyClient().reorderHighlights({ highlightIds: next.map((highlight) => highlight.id) }),
    onMutate: (next) => {
      queryClient.setQueryData(storyKeys.highlights(userId), next);
    },
    onError: (error) => {
      void queryClient.invalidateQueries({ queryKey: storyKeys.highlights(userId) });
      toast.error(errorMessage(error));
    },
  });

  if (editing) return <HighlightEditor highlight={editing} onClose={() => setEditing(undefined)} />;
  return (
    <Dialog
      open
      onOpenChange={(open) => !open && onClose()}
      title={t("highlights.manageTitle")}
      description={t("highlights.manageLead")}
    >
      <Sortable<notes.Highlight>
        items={highlights.data ?? []}
        getKey={(highlight) => highlight.id}
        onReorder={(next) => reorder.mutate(next)}
        className="flex flex-col gap-1.5"
      >
        {(highlight, { grip }) => (
          <div
            className="flex items-center gap-2 rounded-2xl bg-surface-2 p-2"
            data-testid="highlight-row"
          >
            <button
              type="button"
              aria-label={t("highlights.move", { title: highlight.title })}
              className="grid h-12 w-6 shrink-0 place-items-center text-ink-faint hover:text-ink"
              {...grip}
            >
              <GripVertical className="size-5" />
            </button>
            <span className="block size-12 shrink-0 overflow-hidden rounded-full border-2 border-ink bg-surface-3">
              {highlight.cover?.url && (
                <img
                  src={imageUrl(highlight.cover, 48)}
                  alt=""
                  className="size-full object-cover"
                />
              )}
            </span>
            <span className="min-w-0 flex-1">
              <span className="block truncate font-black">{highlight.title}</span>
              <span className="block text-sm text-ink-soft">
                {t("highlights.count", { count: highlight.storyCount })}
              </span>
            </span>
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label={t("highlights.edit", { title: highlight.title })}
              onClick={() => setEditing(highlight)}
            >
              <Pencil className="size-4" />
            </Button>
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label={t("highlights.deleteNamed", { title: highlight.title })}
              className="text-danger"
              onClick={() => setDeleting(highlight)}
            >
              <Trash2 className="size-4" />
            </Button>
          </div>
        )}
      </Sortable>
      <ConfirmDialog
        open={deleting !== undefined}
        onOpenChange={(open) => !open && setDeleting(undefined)}
        title={t("highlights.deleteTitle", { title: deleting?.title ?? "" })}
        description={t("highlights.deleteBody")}
        confirmLabel={t("highlights.delete")}
        danger
        onConfirm={async () => {
          if (deleting) await deleteHighlight(deleting);
          setDeleting(undefined);
        }}
      />
    </Dialog>
  );
}
