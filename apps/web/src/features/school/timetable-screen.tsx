import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { school } from "@tokuly/api";
import {
  CalendarOff,
  Check,
  Eraser,
  MoreHorizontal,
  Pencil,
  Plus,
  Shapes,
  Share2,
  SquarePen,
  Trash2,
  Users,
} from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Chip } from "#/components/ui/card";
import { Dialog } from "#/components/ui/dialog";
import { TextArea, TextField } from "#/components/ui/field";
import { Menu } from "#/components/ui/menu";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { SelectField } from "#/components/ui/select";
import { cn } from "#/lib/cn";
import { keys, refresh, timetableClient, toastError, useSubjects, useTerm, useTerms } from "./api";
import { dayLabel } from "./dates";
import { TimetableGrid } from "./grid";
import { collides, entryFor, type GridEntry, type Slot } from "./grid-model";
import { inkOn, LoadFailed, SchoolPage, Segmented, SubjectPill } from "./parts";
import { SubjectsDialog } from "./subjects-dialog";
import { DayOverridesDialog, ShareDialog, TermDialog } from "./term-dialogs";

const ERASER = "eraser";
const PAINT = ["school", "timetable", "paint"] as const;

type TermData = school.GetTermResponse;

/** What undoing a paint or an erase does: classes to take out again, classes to put back. */
interface UndoInput {
  termId: string;
  removeIds: string[];
  restore: school.TimetableEntry[];
}

const UNDO_TOAST = "school-timetable-undo";

/** A class the server has (not one drawn ahead of its answer). */
const isSaved = (entry: school.TimetableEntry) => !entry.id.startsWith("pending-");

/**
 * The caller's weekly timetable. Looking at it, a class opens its details;
 * in edit mode a subject is picked once and painted into as many slots as
 * are tapped (tapping it again, or the eraser, empties a slot).
 */
export function TimetableScreen({
  termId,
  onTermChange,
}: {
  termId?: string;
  onTermChange(termId: string): void;
}) {
  const { t, i18n } = useTranslation("school");
  const queryClient = useQueryClient();
  const terms = useTerms();
  const termList = terms.data ?? [];
  const shown =
    termList.find((term) => term.id === termId) ??
    termList.find((term) => term.current) ??
    termList[0];
  const detail = useTerm(shown?.id);
  const subjects = useSubjects();

  const [editing, setEditing] = useState(false);
  const [brush, setBrush] = useState<string | undefined>(undefined);
  const [pattern, setPattern] = useState<school.WeekPattern>(school.WeekPattern.EVERY);
  const [dialog, setDialog] = useState<
    "new" | "edit" | "share" | "overrides" | "subjects" | "newSubject" | undefined
  >(undefined);
  const [opened, setOpened] = useState<
    { weekday: school.Weekday; period: school.Period; slot: Slot } | undefined
  >(undefined);

  const term = detail.data?.term ?? shown;
  const entries = detail.data?.entries ?? [];
  const alternating = term?.alternatingWeeks ?? false;
  const activePattern = alternating ? pattern : school.WeekPattern.EVERY;
  const subjectList = subjects.data ?? [];
  const activeBrush = brush ?? subjectList[0]?.id;

  /** Changes the cached grid right away; the server's answer follows. */
  const patchEntries = (
    termKey: string,
    change: (entries: school.TimetableEntry[]) => school.TimetableEntry[],
  ) =>
    queryClient.setQueryData<TermData>(keys.term(termKey), (current) =>
      current ? { ...current, entries: change(current.entries) } : current,
    );
  const settle = (termKey: string) => {
    // Several taps in a row: ask for the grid again only after the last one.
    if (queryClient.isMutating({ mutationKey: PAINT }) <= 1)
      void queryClient.invalidateQueries({ queryKey: keys.term(termKey) });
    void queryClient.invalidateQueries({
      queryKey: ["school", "timetable"],
      predicate: (query) => query.queryKey[2] !== "term",
    });
  };

  /** Offers to take back the paint or erase that just went through (one toast: the last tap is the one that can be undone). */
  const offerUndo = (message: string, input: UndoInput) =>
    toast(message, {
      id: UNDO_TOAST,
      duration: 6000,
      action: {
        label: t("timetable.undo"),
        onClick: (event) => {
          // Keep this toast open: the answer replaces it in place (same id). Letting it close
          // first would make a quick answer update a toast that is already leaving, so the
          // confirmation never showed.
          event.preventDefault();
          if (!undo.isPending) undo.mutate(input);
        },
      },
    });

  const put = useMutation({
    mutationKey: PAINT,
    mutationFn: (input: {
      termId: string;
      weekday: school.Weekday;
      period: number;
      weekPattern: school.WeekPattern;
      subject: school.Subject;
    }) =>
      timetableClient().putTimetableEntries({
        termId: input.termId,
        entries: [
          {
            weekday: input.weekday,
            period: input.period,
            weekPattern: input.weekPattern,
            subjectId: input.subject.id,
          },
        ],
      }),
    onMutate: async (input) => {
      // A refetch that is on its way would paint the old grid over this tap.
      await queryClient.cancelQueries({ queryKey: keys.term(input.termId) });
      const optimistic = {
        id: `pending-${input.weekday}-${input.period}-${input.weekPattern}`,
        termId: input.termId,
        weekday: input.weekday,
        period: input.period,
        weekPattern: input.weekPattern,
        subject: input.subject,
        teacher: "",
        room: "",
        color: "",
        memo: "",
      } as school.TimetableEntry;
      // What this tap pushes out of the slot: kept to put back if the server refuses, and for undo.
      const displaced = (
        queryClient.getQueryData<TermData>(keys.term(input.termId))?.entries ?? []
      ).filter((entry) => collides(entry, input.weekday, input.period, input.weekPattern));
      patchEntries(input.termId, (current) => [
        ...current.filter((entry) => !displaced.includes(entry)),
        optimistic,
      ]);
      return { displaced, optimisticId: optimistic.id };
    },
    // The saved class takes the place of the one drawn ahead of the answer, so it can be tapped away at once.
    onSuccess: (response, input, context) => {
      patchEntries(input.termId, (current) => [
        ...current.filter(
          (entry) =>
            !collides(entry, input.weekday, input.period, input.weekPattern) &&
            !response.replacedEntryIds.includes(entry.id),
        ),
        ...response.entries,
      ]);
      offerUndo(t("timetable.placed", { subject: input.subject.name }), {
        termId: input.termId,
        removeIds: response.entries.map((entry) => entry.id),
        restore: context.displaced.filter(isSaved),
      });
    },
    // Only this tap is taken back (the slot as it was), so taps made since stay on the grid.
    onError: (error, input, context) => {
      if (context)
        patchEntries(input.termId, (current) => [
          ...current.filter((entry) => entry.id !== context.optimisticId),
          ...context.displaced.filter((entry) => !current.includes(entry)),
        ]);
      toastError(error);
    },
    onSettled: (_data, _error, input) => settle(input.termId),
  });
  const erase = useMutation({
    mutationKey: PAINT,
    mutationFn: (input: { termId: string; entryIds: string[] }) =>
      timetableClient().deleteTimetableEntries(input),
    onMutate: async (input) => {
      await queryClient.cancelQueries({ queryKey: keys.term(input.termId) });
      const removed = (
        queryClient.getQueryData<TermData>(keys.term(input.termId))?.entries ?? []
      ).filter((entry) => input.entryIds.includes(entry.id));
      patchEntries(input.termId, (current) => current.filter((entry) => !removed.includes(entry)));
      return { removed };
    },
    onSuccess: (_response, input, context) =>
      offerUndo(t("timetable.cleared"), {
        termId: input.termId,
        removeIds: [],
        restore: context.removed,
      }),
    onError: (error, input, context) => {
      if (context)
        patchEntries(input.termId, (current) => [
          ...current,
          ...context.removed.filter((entry) => !current.some((other) => other.id === entry.id)),
        ]);
      toastError(error);
    },
    onSettled: (_data, _error, input) => settle(input.termId),
  });
  /** Takes the last paint or erase back: what it added goes, what it removed returns with its room, teacher and memo. */
  const undo = useMutation({
    mutationKey: PAINT,
    mutationFn: async (input: UndoInput) => {
      if (input.removeIds.length > 0)
        await timetableClient().deleteTimetableEntries({
          termId: input.termId,
          entryIds: input.removeIds,
        });
      if (input.restore.length > 0) {
        await timetableClient().putTimetableEntries({
          termId: input.termId,
          entries: input.restore.map((entry) => ({
            weekday: entry.weekday,
            period: entry.period,
            weekPattern: entry.weekPattern || school.WeekPattern.EVERY,
            subjectId: entry.subject?.id ?? "",
            teacher: entry.teacher,
            room: entry.room,
            color: entry.color,
            memo: entry.memo,
          })),
        });
      }
    },
    onMutate: async (input) => {
      await queryClient.cancelQueries({ queryKey: keys.term(input.termId) });
      patchEntries(input.termId, (current) => [
        ...current.filter((entry) => !input.removeIds.includes(entry.id)),
        ...input.restore,
      ]);
    },
    onSuccess: () => toast.success(t("timetable.undone"), { id: UNDO_TOAST }),
    onError: (error) => {
      toast.dismiss(UNDO_TOAST);
      toastError(error);
    },
    onSettled: (_data, _error, input) => settle(input.termId),
  });

  const onCell = (weekday: school.Weekday, period: school.Period, slot: Slot | undefined) => {
    if (!term) return;
    if (!editing) {
      if (slot && (slot.every || slot.a || slot.b)) setOpened({ weekday, period, slot });
      else setEditing(true);
      return;
    }
    const inSlot = [slot?.every, slot?.a, slot?.b].filter((entry): entry is GridEntry =>
      Boolean(entry),
    );
    const targets =
      activePattern === school.WeekPattern.EVERY
        ? inSlot
        : inSlot.filter((entry) => collides(entry, weekday, period.number, activePattern));
    const saved = targets
      .filter((entry) => !entry.id.startsWith("pending-"))
      .map((entry) => entry.id);
    const subject = subjectList.find((item) => item.id === activeBrush);
    const same =
      subject &&
      targets.length === 1 &&
      targets[0]?.subject?.id === subject.id &&
      (targets[0]?.weekPattern || school.WeekPattern.EVERY) === activePattern;
    if (activeBrush === ERASER || same) {
      if (saved.length > 0) erase.mutate({ termId: term.id, entryIds: saved });
      return;
    }
    if (!subject) {
      setDialog("newSubject");
      return;
    }
    put.mutate({
      termId: term.id,
      weekday,
      period: period.number,
      weekPattern: activePattern,
      subject,
    });
  };

  if (terms.isPending) {
    return (
      <SchoolPage section="timetable" wide>
        <Skeleton className="h-96" />
      </SchoolPage>
    );
  }
  if (terms.isError) {
    return (
      <SchoolPage section="timetable">
        <LoadFailed />
      </SchoolPage>
    );
  }

  if (!term) {
    return (
      <SchoolPage section="timetable">
        <EmptyState
          emoji="🗓️"
          title={t("timetable.empty")}
          body={t("timetable.emptyBody")}
          action={
            <div className="flex flex-col items-center gap-2">
              <Button icon={<Plus className="size-4" />} onClick={() => setDialog("new")}>
                {t("terms.create")}
              </Button>
              <Link to="/school/friends" className="text-sm font-bold text-brand">
                {t("timetable.copyFromFriend")}
              </Link>
            </div>
          }
        />
        <TermDialog
          open={dialog === "new"}
          onOpenChange={(open) => setDialog(open ? "new" : undefined)}
          onSaved={(saved) => onTermChange(saved.id)}
        />
      </SchoolPage>
    );
  }

  const audienceLabel =
    term.audience === school.TimetableAudience.FRIENDS
      ? t("share.friends")
      : term.audience === school.TimetableAudience.CHOSEN
        ? t("share.chosenCount", { count: term.sharedWith.length })
        : t("share.nobody");

  return (
    <SchoolPage
      section="timetable"
      wide
      actions={
        <>
          <Button
            size="sm"
            variant={editing ? "lime" : "secondary"}
            icon={editing ? <Check className="size-4" /> : <Pencil className="size-4" />}
            onClick={() => setEditing(!editing)}
          >
            {editing ? t("timetable.done") : t("timetable.edit")}
          </Button>
          <Menu
            trigger={
              <Button size="icon-sm" variant="ghost" aria-label={t("timetable.menu")}>
                <MoreHorizontal className="size-5" />
              </Button>
            }
            actions={[
              { label: t("terms.edit"), icon: <SquarePen />, onSelect: () => setDialog("edit") },
              {
                label: t("subjects.title"),
                icon: <Shapes />,
                onSelect: () => setDialog("subjects"),
              },
              { label: t("share.title"), icon: <Share2 />, onSelect: () => setDialog("share") },
              {
                label: t("overrides.title"),
                icon: <CalendarOff />,
                onSelect: () => setDialog("overrides"),
              },
              { label: t("terms.create"), icon: <Plus />, onSelect: () => setDialog("new") },
            ]}
          />
        </>
      }
    >
      <div className="lg:grid lg:grid-cols-[minmax(0,1fr)_20rem] lg:items-start lg:gap-6">
        <div className="min-w-0">
          <div className="mb-3 flex flex-wrap items-center gap-2">
            {termList.length > 1 ? (
              <SelectField
                className="min-w-44 flex-1 sm:max-w-xs"
                value={term.id}
                onChange={onTermChange}
                options={termList.map((item) => ({
                  value: item.id,
                  label: item.name,
                  hint: item.current ? t("terms.currentBadge") : undefined,
                }))}
              />
            ) : (
              <h2 className="min-w-0 flex-1 truncate text-lg font-black">{term.name}</h2>
            )}
            <button
              type="button"
              onClick={() => setDialog("share")}
              className="inline-flex h-8 items-center gap-1.5 rounded-full bg-surface-2 px-3 text-xs font-bold text-ink-soft hover:text-ink"
            >
              <Share2 className="size-3.5" />
              {audienceLabel}
            </button>
          </div>
          <p className="mb-3 flex flex-wrap items-center gap-1.5 px-1 text-xs font-bold text-ink-faint">
            <span>
              {dayLabel(term.startDate, i18n.language)} 〜 {dayLabel(term.endDate, i18n.language)}
            </span>
            {!term.current && <Chip>{t("terms.notCurrent")}</Chip>}
            {alternating && <Chip>{t("timetable.alternating")}</Chip>}
            {term.copiedFrom && (
              <Chip>{t("timetable.copiedFrom", { name: term.copiedFrom.displayName })}</Chip>
            )}
          </p>

          {editing && (
            <div className="sticker mb-3 flex flex-col gap-3 rounded-card bg-surface p-3">
              <p className="px-1 text-sm font-bold text-ink-soft">{t("timetable.editHint")}</p>
              <div
                role="radiogroup"
                aria-label={t("timetable.brush")}
                className="flex flex-wrap gap-1.5"
              >
                {subjectList.map((subject) => (
                  <BrushButton
                    key={subject.id}
                    active={activeBrush === subject.id}
                    onClick={() => setBrush(subject.id)}
                    label={subject.name}
                  >
                    <span
                      className="inline-flex h-7 items-center gap-1 rounded-full border-2 border-ink px-2.5 text-sm font-black"
                      style={{ background: subject.color, color: inkOn(subject.color) }}
                    >
                      {subject.emoji && <span aria-hidden>{subject.emoji}</span>}
                      {subject.name}
                    </span>
                  </BrushButton>
                ))}
                <BrushButton
                  active={activeBrush === ERASER}
                  onClick={() => setBrush(ERASER)}
                  label={t("timetable.eraser")}
                >
                  <span className="inline-flex h-7 items-center gap-1 rounded-full border-2 border-ink bg-surface px-2.5 text-sm font-black">
                    <Eraser className="size-3.5" />
                    {t("timetable.eraser")}
                  </span>
                </BrushButton>
                <button
                  type="button"
                  onClick={() => setDialog("newSubject")}
                  className="inline-flex h-9 items-center gap-1 rounded-full border-2 border-dashed border-ink-faint px-3 text-sm font-bold text-ink-soft hover:text-ink"
                >
                  <Plus className="size-4" />
                  {t("subjects.add")}
                </button>
              </div>
              {alternating && (
                <Segmented
                  label={t("timetable.pattern")}
                  value={pattern}
                  onChange={(next: school.WeekPattern) => setPattern(next)}
                  options={[
                    { value: school.WeekPattern.EVERY, label: t("timetable.everyWeek") },
                    { value: school.WeekPattern.A, label: t("timetable.weekA") },
                    { value: school.WeekPattern.B, label: t("timetable.weekB") },
                  ]}
                />
              )}
            </div>
          )}

          {detail.isPending ? (
            <Skeleton className="h-96" />
          ) : (
            <TimetableGrid
              layout={term.layout}
              entries={entries}
              onCell={onCell}
              editing={editing}
              highlightToday={term.current}
            />
          )}
          {!editing && entries.length === 0 && !detail.isPending && (
            <p className="mt-3 text-center text-sm font-bold text-ink-soft">
              {t("timetable.emptyGrid")}
            </p>
          )}
        </div>

        <aside className="mt-6 flex flex-col gap-3 lg:mt-0">
          <Link
            to="/school/friends"
            className="pressable sticker flex items-center gap-3 rounded-card bg-surface p-4"
          >
            <span className="grid size-11 shrink-0 place-items-center rounded-2xl border-2 border-ink bg-sky text-[#17122b]">
              <Users className="size-6" />
            </span>
            <span className="min-w-0">
              <span className="block font-black">{t("friends.title")}</span>
              <span className="block text-sm text-ink-soft">{t("friends.lead")}</span>
            </span>
          </Link>
          <Link
            to="/school/friends"
            search={{ tab: "free" }}
            className="pressable sticker flex items-center gap-3 rounded-card bg-surface p-4"
          >
            <span className="grid size-11 shrink-0 place-items-center rounded-2xl border-2 border-ink bg-lime text-2xl text-[#17122b]">
              ☕
            </span>
            <span className="min-w-0">
              <span className="block font-black">{t("free.title")}</span>
              <span className="block text-sm text-ink-soft">{t("free.lead")}</span>
            </span>
          </Link>
        </aside>
      </div>

      <TermDialog
        open={dialog === "new"}
        onOpenChange={(open) => setDialog(open ? "new" : undefined)}
        onSaved={(saved) => onTermChange(saved.id)}
      />
      <TermDialog
        open={dialog === "edit"}
        onOpenChange={(open) => setDialog(open ? "edit" : undefined)}
        term={term}
      />
      {dialog === "share" && (
        <ShareDialog open onOpenChange={() => setDialog(undefined)} term={term} />
      )}
      {dialog === "overrides" && (
        <DayOverridesDialog
          open
          onOpenChange={() => setDialog(undefined)}
          term={term}
          overrides={detail.data?.dayOverrides ?? []}
        />
      )}
      <SubjectsDialog
        open={dialog === "subjects" || dialog === "newSubject"}
        startNew={dialog === "newSubject"}
        onOpenChange={(open) => !open && setDialog(undefined)}
        onCreated={(subject) => setBrush(subject.id)}
      />
      {opened && (
        <EntryDialog
          term={term}
          weekday={opened.weekday}
          period={opened.period}
          slot={opened.slot}
          onClose={() => setOpened(undefined)}
        />
      )}
    </SchoolPage>
  );
}

function BrushButton({
  active,
  onClick,
  label,
  children,
}: {
  active: boolean;
  onClick(): void;
  label: string;
  children: React.ReactNode;
}) {
  return (
    // biome-ignore lint/a11y/useSemanticElements: the brushes are buttons that behave as one radio group.
    <button
      type="button"
      role="radio"
      aria-checked={active}
      aria-label={label}
      onClick={onClick}
      className={cn(
        "rounded-full p-0.5 transition-transform",
        active ? "scale-105 bg-ink" : "opacity-70 hover:opacity-100",
      )}
    >
      {children}
    </button>
  );
}

/** One slot up close: who teaches where, a private memo, and removing the class. */
function EntryDialog({
  term,
  weekday,
  period,
  slot,
  onClose,
}: {
  term: school.Term;
  weekday: school.Weekday;
  period: school.Period;
  slot: Slot;
  onClose(): void;
}) {
  const { t } = useTranslation("school");
  const queryClient = useQueryClient();
  const choices = [slot.every, slot.a, slot.b].filter((entry): entry is GridEntry =>
    Boolean(entry),
  );
  const [pattern, setPattern] = useState<school.WeekPattern>(
    choices[0]?.weekPattern || school.WeekPattern.EVERY,
  );
  const entry = entryFor(slot, pattern) ?? choices[0];
  const [form, setForm] = useState({
    teacher: entry?.teacher ?? "",
    room: entry?.room ?? "",
    memo: entry?.memo ?? "",
  });
  const pick = (next: school.WeekPattern) => {
    const other = entryFor(slot, next);
    setPattern(next);
    setForm({ teacher: other?.teacher ?? "", room: other?.room ?? "", memo: other?.memo ?? "" });
  };

  const save = useMutation({
    mutationFn: () =>
      timetableClient().putTimetableEntries({
        termId: term.id,
        entries: [
          {
            weekday,
            period: period.number,
            weekPattern: entry?.weekPattern || school.WeekPattern.EVERY,
            subjectId: entry?.subject?.id ?? "",
            color: entry?.color ?? "",
            ...form,
          },
        ],
      }),
    onSuccess: () => {
      refresh(queryClient, "timetable");
      onClose();
    },
    onError: toastError,
  });
  const remove = useMutation({
    mutationFn: () =>
      timetableClient().deleteTimetableEntries({
        termId: term.id,
        entryIds: entry ? [entry.id] : [],
      }),
    onSuccess: () => {
      refresh(queryClient, "timetable");
      onClose();
    },
    onError: toastError,
  });
  if (!entry) return null;

  return (
    <Dialog
      open
      onOpenChange={(open) => !open && onClose()}
      title={t("timetable.slotLabel", {
        weekday: t(`weekdays.long.${weekday}`),
        period: period.number,
      })}
      footer={
        <>
          <Button
            variant="ghost"
            className="mr-auto text-danger"
            icon={<Trash2 className="size-4" />}
            loading={remove.isPending}
            onClick={() => remove.mutate()}
          >
            {t("timetable.removeClass")}
          </Button>
          <Button loading={save.isPending} onClick={() => save.mutate()}>
            {t("common:actions.save")}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-4">
        {choices.length > 1 && (
          <Segmented
            label={t("timetable.pattern")}
            value={pattern}
            onChange={(next: school.WeekPattern) => pick(next)}
            options={choices.map((choice) => ({
              value: choice.weekPattern,
              label:
                choice.weekPattern === school.WeekPattern.B
                  ? t("timetable.weekB")
                  : t("timetable.weekA"),
            }))}
          />
        )}
        <div className="flex items-center gap-2">
          <SubjectPill subject={entry.subject} className="h-8 px-3 text-sm" />
          {entry.weekPattern === school.WeekPattern.A && <Chip>{t("timetable.weekA")}</Chip>}
          {entry.weekPattern === school.WeekPattern.B && <Chip>{t("timetable.weekB")}</Chip>}
        </div>
        <div className="grid grid-cols-2 gap-3">
          <TextField
            label={t("subjects.teacher")}
            value={form.teacher}
            maxLength={40}
            placeholder={entry.subject?.teacher}
            onChange={(event) => setForm({ ...form, teacher: event.target.value })}
          />
          <TextField
            label={t("subjects.room")}
            value={form.room}
            maxLength={40}
            placeholder={entry.subject?.room}
            onChange={(event) => setForm({ ...form, room: event.target.value })}
          />
        </div>
        <TextArea
          label={t("timetable.memo")}
          hint={t("timetable.memoHint")}
          value={form.memo}
          maxLength={500}
          onChange={(event) => setForm({ ...form, memo: event.target.value })}
        />
      </div>
    </Dialog>
  );
}
