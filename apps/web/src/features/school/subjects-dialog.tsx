import { useMutation, useQueryClient } from "@tanstack/react-query";
import type { school } from "@tokuly/api";
import { ArchiveRestore, Check, Plus, Trash2 } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { Dialog } from "#/components/ui/dialog";
import { TextField } from "#/components/ui/field";
import { EmptyState } from "#/components/ui/page";
import { cn } from "#/lib/cn";
import { refresh, timetableClient, toastError, useSubjects } from "./api";
import { RowsSkeleton, SubjectPill } from "./parts";

/** The colours offered for a subject (the server accepts any `#rrggbb`). */
export const SUBJECT_COLORS = ["#ff7a59", "#ffd23f", "#c6f24e", "#3ddc97", "#4fc3ff", "#7c5cff", "#ff5c9d", "#b98cff", "#ffa94d", "#63e6be", "#74c0fc", "#ced4da"];

interface Draft {
  id?: string;
  name: string;
  shortName: string;
  emoji: string;
  color: string;
  teacher: string;
  room: string;
}

const blank = (color: string): Draft => ({ name: "", shortName: "", emoji: "", color, teacher: "", room: "" });

/** The caller's subjects: add, change (name, short form, emoji, colour, usual teacher and room), remove, bring back. */
export function SubjectsDialog({
  open,
  onOpenChange,
  startNew,
  onCreated,
}: {
  open: boolean;
  onOpenChange(open: boolean): void;
  /** Open straight on the "new subject" form, and close once it is saved. */
  startNew?: boolean;
  onCreated?(subject: school.Subject): void;
}) {
  const { t } = useTranslation("school");
  const queryClient = useQueryClient();
  const subjects = useSubjects(true);
  const [draft, setDraft] = useState<Draft | undefined>(undefined);
  const [removing, setRemoving] = useState<school.Subject | undefined>(undefined);
  const list = subjects.data ?? [];
  const nextColor = SUBJECT_COLORS[list.length % SUBJECT_COLORS.length] ?? "#7c5cff";
  // Opening on the form: a draft appears once per opening.
  const [seeded, setSeeded] = useState(false);
  if (open && startNew && !seeded) {
    setSeeded(true);
    setDraft(blank(nextColor));
  }
  if (!open && seeded) setSeeded(false);

  const changed = () => refresh(queryClient, "subjects", "timetable", "exams", "assignments");
  const save = useMutation({
    mutationFn: async (input: Draft) => {
      if (input.id) await timetableClient().updateSubject({ subjectId: input.id, ...input });
      else return (await timetableClient().createSubject(input)).subject;
    },
    onSuccess: (created) => {
      changed();
      setDraft(undefined);
      if (created) onCreated?.(created);
      if (created && startNew) onOpenChange(false);
    },
    onError: toastError,
  });
  const remove = useMutation({
    mutationFn: (subject: school.Subject) => timetableClient().deleteSubject({ subjectId: subject.id }),
    onSuccess: (response) => {
      toast.success(response.archived ? t("subjects.archivedToast") : t("subjects.deletedToast"));
      changed();
      setRemoving(undefined);
      setDraft(undefined);
    },
    onError: toastError,
  });
  const restore = useMutation({
    mutationFn: (subject: school.Subject) => timetableClient().updateSubject({ subjectId: subject.id, archived: false }),
    onSuccess: changed,
    onError: toastError,
  });

  const close = (next: boolean) => {
    if (!next) setDraft(undefined);
    onOpenChange(next);
  };

  return (
    <>
      <Dialog
        open={open}
        onOpenChange={close}
        title={draft ? (draft.id ? t("subjects.edit") : t("subjects.add")) : t("subjects.title")}
        footer={
          draft ? (
            <>
              {draft.id && (
                <Button variant="ghost" className="mr-auto text-danger" icon={<Trash2 className="size-4" />} onClick={() => setRemoving(list.find((subject) => subject.id === draft.id))}>
                  {t("common:actions.delete")}
                </Button>
              )}
              <Button variant="ghost" onClick={() => setDraft(undefined)}>
                {t("common:actions.back")}
              </Button>
              <Button loading={save.isPending} disabled={!draft.name.trim()} onClick={() => save.mutate(draft)}>
                {t("common:actions.save")}
              </Button>
            </>
          ) : (
            <Button icon={<Plus className="size-4" />} onClick={() => setDraft(blank(nextColor))}>
              {t("subjects.add")}
            </Button>
          )
        }
      >
        {draft ? (
          <form
            className="flex flex-col gap-4"
            onSubmit={(event) => {
              event.preventDefault();
              if (draft.name.trim()) save.mutate(draft);
            }}
          >
            <TextField label={t("subjects.name")} value={draft.name} maxLength={40} placeholder={t("subjects.namePlaceholder")} onChange={(event) => setDraft({ ...draft, name: event.target.value })} autoFocus />
            <div className="grid grid-cols-2 gap-3">
              <TextField label={t("subjects.shortName")} value={draft.shortName} maxLength={8} placeholder={t("subjects.shortNamePlaceholder")} onChange={(event) => setDraft({ ...draft, shortName: event.target.value })} />
              <TextField label={t("subjects.emoji")} value={draft.emoji} maxLength={8} placeholder="📐" onChange={(event) => setDraft({ ...draft, emoji: event.target.value })} />
            </div>
            <div>
              <p className="px-1 pb-1.5 text-sm font-bold text-ink-soft">{t("subjects.color")}</p>
              <div role="radiogroup" aria-label={t("subjects.color")} className="flex flex-wrap gap-2">
                {SUBJECT_COLORS.map((color) => (
                  // biome-ignore lint/a11y/useSemanticElements: colour swatches are buttons.
                  <button
                    key={color}
                    type="button"
                    role="radio"
                    aria-checked={draft.color === color}
                    aria-label={color}
                    onClick={() => setDraft({ ...draft, color })}
                    className={cn("grid size-9 place-items-center rounded-full border-2 text-[#17122b]", draft.color === color ? "border-ink shadow-pop-sm" : "border-transparent")}
                    style={{ background: color }}
                  >
                    {draft.color === color && <Check className="size-4" />}
                  </button>
                ))}
              </div>
            </div>
            <div className="grid grid-cols-2 gap-3">
              <TextField label={t("subjects.teacher")} value={draft.teacher} maxLength={40} onChange={(event) => setDraft({ ...draft, teacher: event.target.value })} />
              <TextField label={t("subjects.room")} value={draft.room} maxLength={40} onChange={(event) => setDraft({ ...draft, room: event.target.value })} />
            </div>
            <button type="submit" hidden />
          </form>
        ) : subjects.isPending ? (
          <RowsSkeleton className="h-12" />
        ) : list.length === 0 ? (
          <EmptyState emoji="📚" title={t("subjects.empty")} body={t("subjects.emptyBody")} />
        ) : (
          <ul className="-mx-2 flex flex-col">
            {list.map((subject) => (
              <li key={subject.id} className="flex items-center gap-2">
                <button
                  type="button"
                  onClick={() => setDraft({ id: subject.id, name: subject.name, shortName: subject.shortName, emoji: subject.emoji, color: subject.color, teacher: subject.teacher, room: subject.room })}
                  className={cn("flex min-w-0 flex-1 items-center gap-3 rounded-2xl px-2 py-2.5 text-left transition-colors hover:bg-surface-2", subject.archived && "opacity-60")}
                >
                  <SubjectPill subject={subject} />
                  <span className="min-w-0 flex-1 truncate text-sm text-ink-soft">{[subject.teacher, subject.room].filter(Boolean).join(" · ")}</span>
                  {subject.archived && <span className="shrink-0 text-xs font-bold text-ink-faint">{t("subjects.archived")}</span>}
                </button>
                {subject.archived && (
                  <Button size="icon-sm" variant="ghost" aria-label={t("subjects.restore", { name: subject.name })} onClick={() => restore.mutate(subject)}>
                    <ArchiveRestore className="size-4" />
                  </Button>
                )}
              </li>
            ))}
          </ul>
        )}
      </Dialog>
      <ConfirmDialog
        open={Boolean(removing)}
        onOpenChange={(next) => !next && setRemoving(undefined)}
        title={t("subjects.deleteTitle", { name: removing?.name ?? "" })}
        description={t("subjects.deleteBody")}
        confirmLabel={t("common:actions.delete")}
        danger
        loading={remove.isPending}
        onConfirm={() => removing && remove.mutate(removing)}
      />
    </>
  );
}
