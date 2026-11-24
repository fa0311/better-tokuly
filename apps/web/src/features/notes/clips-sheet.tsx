import { useMutation, useQueryClient } from "@tanstack/react-query";
import type { notes } from "@tokuly/api";
import { Check, Globe, Lock, Plus } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Dialog } from "#/components/ui/dialog";
import { LoadMore } from "#/components/ui/load-more";
import { Skeleton } from "#/components/ui/page";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { clipClient, clipKeys, storeClip, useClips } from "./clips-api";
import { ClipForm } from "./clips-form";

/**
 * The "add to clip" sheet of a note: the caller's clips, each with whether
 * it holds the note (tap to put it in or take it out), and a new clip.
 */
export function AddToClipSheet({ note, onClose }: { note: notes.Note; onClose(): void }) {
  const { t } = useTranslation("clips");
  const queryClient = useQueryClient();
  const clips = useClips("", note.id);
  const [creating, setCreating] = useState(false);
  const toggle = useMutation({
    mutationFn: async (clip: notes.Clip) => {
      const request = { clipId: clip.id, noteId: note.id };
      return clip.containsNote
        ? (await clipClient().removeNoteFromClip(request)).clip!
        : (await clipClient().addNoteToClip(request)).clip!;
    },
    onSuccess: (updated) => {
      storeClip(queryClient, updated);
      toast.success(
        t(updated.containsNote ? "sheet.added" : "sheet.removed", { name: updated.name }),
      );
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "clips" })),
  });
  const rows = clips.data?.pages.flatMap((page) => page.clips) ?? [];

  return (
    <Dialog
      open
      onOpenChange={(open) => !open && onClose()}
      title={creating ? t("form.createTitle") : t("sheet.title")}
      description={creating ? undefined : t("sheet.lead")}
    >
      {creating ? (
        <ClipForm
          onCancel={() => setCreating(false)}
          onSaved={async (clip) => {
            setCreating(false);
            await toggle.mutateAsync(clip).catch(() => undefined);
            void queryClient.invalidateQueries({ queryKey: clipKeys.list("", note.id) });
          }}
        />
      ) : (
        <div className="flex flex-col gap-2">
          {clips.isPending && <Skeleton className="h-14 rounded-2xl" />}
          {clips.isSuccess && rows.length === 0 && (
            <p className="rounded-2xl bg-surface-2 px-4 py-3 text-sm font-bold text-ink-soft">
              {t("sheet.empty")}
            </p>
          )}
          <ul className="flex flex-col gap-1">
            {rows.map((clip) => {
              const Audience = clip.public ? Globe : Lock;
              return (
                <li key={clip.id}>
                  <button
                    type="button"
                    aria-pressed={clip.containsNote}
                    disabled={toggle.isPending}
                    onClick={() => toggle.mutate(clip)}
                    className="flex w-full items-center gap-3 rounded-2xl px-3 py-2.5 text-left hover:bg-surface-2 disabled:opacity-60"
                  >
                    <span
                      className={cn(
                        "grid size-7 shrink-0 place-items-center rounded-full border-2",
                        clip.containsNote ? "border-ink bg-lime text-[#17122b]" : "border-line",
                      )}
                    >
                      {clip.containsNote && <Check className="size-4" strokeWidth={3} />}
                    </span>
                    <span className="min-w-0 flex-1">
                      <span className="block truncate font-bold">{clip.name}</span>
                      <span className="flex items-center gap-1.5 text-xs text-ink-faint">
                        <Audience className="size-3" aria-hidden />
                        {clip.public ? t("public") : t("private")} ·{" "}
                        {t("noteCount", { count: clip.noteCount })}
                      </span>
                    </span>
                  </button>
                </li>
              );
            })}
          </ul>
          <LoadMore query={clips} />
          <Button
            variant="secondary"
            icon={<Plus className="size-4" />}
            onClick={() => setCreating(true)}
          >
            {t("sheet.new")}
          </Button>
        </div>
      )}
    </Dialog>
  );
}
