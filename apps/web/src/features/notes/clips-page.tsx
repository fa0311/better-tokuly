import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { clipKeys } from "@tokuly/core/notes";
import { Globe, Lock, Pencil, Trash2, X } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { Dialog } from "#/components/ui/dialog";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { errorMessage } from "#/lib/errors";
import { useMe } from "#/stores/session";
import { clipClient, storeClip, useClip, useClipNotes } from "./clips-api";
import { ClipForm } from "./clips-form";
import { NoteCard } from "./note-card";

/** One clip: what it is, the notes in it the reader may see, and for its owner editing it. */
export function ClipPage({ clipId }: { clipId: string }) {
  const { t } = useTranslation("clips");
  const me = useMe();
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const clip = useClip(clipId);
  const notes = useClipNotes(clipId);
  const [editing, setEditing] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const own = Boolean(clip.data && clip.data.owner?.id === me?.user?.id);

  const remove = useMutation({
    mutationFn: () => clipClient().deleteClip({ clipId }),
    onSuccess: () => {
      toast.success(t("deleted"));
      void queryClient.invalidateQueries({ queryKey: clipKeys.lists });
      void navigate({ to: "/notes/clips" });
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "clips" })),
  });
  const takeOut = useMutation({
    mutationFn: async (noteId: string) =>
      (await clipClient().removeNoteFromClip({ clipId, noteId })).clip!,
    onSuccess: (updated) => {
      storeClip(queryClient, updated);
      toast.success(t("removedNote"));
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "clips" })),
  });

  if (clip.isError) {
    return (
      <Page title={t("title")} back="/notes/clips">
        <EmptyState emoji="📎" title={t("notFound")} body={t("notFoundBody")} />
      </Page>
    );
  }
  const data = clip.data;
  const rows = notes.data?.pages.flatMap((page) => page.notes) ?? [];
  const Audience = data?.public ? Globe : Lock;
  return (
    <Page
      title={data?.name ?? t("title")}
      back={own ? "/notes/clips" : "/notes"}
      actions={
        own && (
          <>
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label={t("edit")}
              onClick={() => setEditing(true)}
            >
              <Pencil className="size-5" />
            </Button>
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label={t("delete")}
              onClick={() => setDeleting(true)}
            >
              <Trash2 className="size-5" />
            </Button>
          </>
        )
      }
    >
      {!data ? (
        <Skeleton className="h-24 rounded-card" />
      ) : (
        <section className="sticker rounded-card bg-surface p-4">
          <h2 className="text-xl font-black">{data.name}</h2>
          {data.description && (
            <p className="mt-1 whitespace-pre-wrap text-sm text-ink-soft">{data.description}</p>
          )}
          <p className="mt-2 flex flex-wrap items-center gap-2 text-xs font-bold text-ink-faint">
            <Audience className="size-3.5" aria-hidden />
            {data.public ? t("public") : t("private")}
            <span>{t("noteCount", { count: data.noteCount })}</span>
            {data.owner && !own && (
              <Link
                to="/notes/clips"
                search={{ user: data.owner.handle }}
                className="hover:underline"
              >
                {t("by", { handle: data.owner.handle })}
              </Link>
            )}
          </p>
        </section>
      )}
      <div className="mt-4 flex flex-col gap-2.5" data-testid="clip-notes">
        {notes.isPending &&
          Array.from({ length: 3 }, (_, index) => (
            <Skeleton key={index} className="h-28 rounded-[1.25rem]" />
          ))}
        {notes.isSuccess && rows.length === 0 && (
          <EmptyState
            emoji="🗒️"
            title={t("notesEmpty")}
            body={own ? t("notesEmptyBody") : undefined}
          />
        )}
        {rows.map((note) => (
          <div key={note.id}>
            <NoteCard note={note} />
            {own && (
              <div className="mt-1 flex justify-end px-1">
                <Button
                  variant="ghost"
                  size="sm"
                  icon={<X className="size-4" />}
                  loading={takeOut.isPending && takeOut.variables === note.id}
                  onClick={() => takeOut.mutate(note.id)}
                >
                  {t("removeNote")}
                </Button>
              </div>
            )}
          </div>
        ))}
        <LoadMore query={notes} showEnd={rows.length > 0} />
      </div>
      {data && (
        <Dialog open={editing} onOpenChange={setEditing} title={t("form.editTitle")}>
          <ClipForm
            clip={data}
            onCancel={() => setEditing(false)}
            onSaved={() => setEditing(false)}
          />
        </Dialog>
      )}
      <ConfirmDialog
        open={deleting}
        onOpenChange={setDeleting}
        title={t("deleteTitle")}
        description={t("deleteBody")}
        confirmLabel={t("delete")}
        danger
        loading={remove.isPending}
        onConfirm={() => remove.mutate()}
      />
    </Page>
  );
}
