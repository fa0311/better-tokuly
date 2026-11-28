import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { fromDate, school, toDate } from "@tokuly/api";
import {
  addDays,
  dueItems,
  examCountdown,
  fromLocalInput,
  momentLabel,
  refreshSchool,
  toLocalInput,
  urgencyKey,
} from "@tokuly/core/school";
import { BellOff, Check, Trash2 } from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { Dialog } from "#/components/ui/dialog";
import { TextArea, TextField } from "#/components/ui/field";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState } from "#/components/ui/page";
import { SwitchRow } from "#/components/ui/switch";
import { useAccountTimeZone, useAccountToday, zonedMoment } from "#/lib/account-time";
import { cn } from "#/lib/cn";
import { useTranslation } from "#/lib/use-translation";
import { assignmentClient, toastError, useAssignments, useExams } from "./api";

import { RowsSkeleton, SubjectPill } from "./parts";
import { SubjectSelect } from "./subject-select";

const URGENCY_STYLE: Partial<Record<school.AssignmentUrgency, string>> = {
  [school.AssignmentUrgency.OVERDUE]: "bg-danger text-white",
  [school.AssignmentUrgency.TODAY]: "bg-coral text-white",
  [school.AssignmentUrgency.TOMORROW]: "bg-sun text-[#17122b]",
  [school.AssignmentUrgency.THIS_WEEK]: "bg-sky text-[#17122b]",
};

/** An exam's countdown in words: "あと5日" before it, "テスト期間中 (あと3日)" during it. */
export function useExamCountdown() {
  const { i18n } = useTranslation("school");
  return (exam: Pick<school.Exam, "status" | "daysUntilStart" | "daysLeft">) =>
    examCountdown(i18n, exam);
}

/** One assignment: tick it off, see what it is for and when it is due, open it to change it. */
export function AssignmentRow({
  assignment,
  onOpen,
}: {
  assignment: school.Assignment;
  onOpen?(assignment: school.Assignment): void;
}) {
  const { t, i18n } = useTranslation("school");
  const queryClient = useQueryClient();
  const toggle = useMutation({
    mutationFn: (done: boolean) =>
      assignmentClient().updateAssignment({ assignmentId: assignment.id, done }),
    onSuccess: (_response, done) => {
      if (done) toast.success(t("assignments.doneToast"));
      refreshSchool(queryClient, "assignments");
    },
    onError: toastError,
  });
  const due = toDate(assignment.dueAt);
  const zone = useAccountTimeZone();
  const urgency = urgencyKey(assignment.urgency);
  const done = toggle.isPending ? toggle.variables : assignment.done;
  const text = (
    <>
      <span className={cn("block truncate font-bold", done && "text-ink-faint line-through")}>
        {assignment.title}
      </span>
      <span className="mt-0.5 flex flex-wrap items-center gap-1.5 text-xs font-bold text-ink-soft">
        {assignment.subject && (
          <SubjectPill subject={assignment.subject} className="h-5 border px-1.5 text-[11px]" />
        )}
        {!done && urgency && (
          <span
            className={cn(
              "rounded-full px-2 py-0.5 text-[11px] font-black",
              URGENCY_STYLE[assignment.urgency],
            )}
          >
            {t(`assignments.urgency.${urgency}`)}
          </span>
        )}
        {due && (
          <span>{t("assignments.dueAt", { when: momentLabel(due, i18n.language, zone) })}</span>
        )}
        {!assignment.remind && !done && (
          <BellOff className="size-3.5" aria-label={t("assignments.remindOff")} />
        )}
      </span>
    </>
  );
  return (
    <li className="flex items-center gap-3 rounded-2xl bg-surface-2 px-3 py-2.5">
      <button
        type="button"
        aria-pressed={done}
        aria-label={t(done ? "assignments.markOpen" : "assignments.markDone", {
          title: assignment.title,
        })}
        onClick={() => toggle.mutate(!done)}
        className={cn(
          "pressable grid size-8 shrink-0 place-items-center rounded-full border-2 border-ink",
          done ? "bg-lime text-[#17122b]" : "bg-surface",
        )}
      >
        {done && <Check className="size-5" />}
      </button>
      {onOpen ? (
        <button
          type="button"
          onClick={() => onOpen(assignment)}
          className="min-w-0 flex-1 text-left"
        >
          {text}
        </button>
      ) : (
        <Link to="/school/assignments" className="min-w-0 flex-1">
          {text}
        </Link>
      )}
    </li>
  );
}

/**
 * "What is due": the exams that are on or coming up and the open
 * assignments, soonest first, in one list.
 */
export function DueList({ limit }: { limit?: number }) {
  const { t, i18n } = useTranslation("school");
  const exams = useExams();
  const assignments = useAssignments(school.AssignmentFilter.OPEN);
  const countdown = useExamCountdown();
  const zone = useAccountTimeZone();
  if (exams.isPending || assignments.isPending) return <RowsSkeleton rows={2} />;

  const items = dueItems(exams.data ?? [], assignments.data?.assignments ?? [], zone);
  if (items.length === 0)
    return (
      <p className="rounded-card bg-surface-2 px-4 py-5 text-center text-sm font-bold text-ink-soft">
        {t("due.empty")}
      </p>
    );
  const shown = limit ? items.slice(0, limit) : items;

  return (
    <>
      <ul className="flex flex-col gap-1.5" aria-label={t("due.title")}>
        {shown.map((item) =>
          item.assignment ? (
            <AssignmentRow key={item.assignment.id} assignment={item.assignment} />
          ) : (
            <li key={item.exam.id}>
              <Link
                to="/school/exams/$examId"
                params={{ examId: item.exam.id }}
                className="flex items-center gap-3 rounded-2xl bg-surface-2 px-3 py-2.5 transition-colors hover:bg-surface-3"
              >
                <span className="grid size-8 shrink-0 place-items-center rounded-full border-2 border-ink bg-sun text-base">
                  📝
                </span>
                <span className="min-w-0 flex-1">
                  <span className="block truncate font-bold">{item.exam.name}</span>
                  <span className="block text-xs font-bold text-ink-soft">
                    {i18n.language.startsWith("ja")
                      ? `${item.exam.startDate.slice(5).replace("-", "/")} 〜`
                      : item.exam.startDate}
                  </span>
                </span>
                <span
                  className={cn(
                    "shrink-0 rounded-full px-2.5 py-1 text-xs font-black",
                    item.exam.status === school.ExamStatus.ONGOING
                      ? "bg-live text-white"
                      : "bg-ink text-surface",
                  )}
                >
                  {countdown(item.exam)}
                </span>
              </Link>
            </li>
          ),
        )}
        {limit !== undefined && items.length > limit && (
          <li className="px-1 text-right text-sm font-bold text-brand">
            <Link to="/school/exams">{t("due.more", { count: items.length - limit })}</Link>
          </li>
        )}
      </ul>
      {limit === undefined && <LoadMore query={assignments.paging} />}
    </>
  );
}

/** Adds an assignment or changes one: title, subject, due time, memo, reminders; removing it. */
export function AssignmentDialog({
  assignment,
  onClose,
}: {
  /** The assignment to change; a new one when unset. */ assignment?: school.Assignment;
  onClose(): void;
}) {
  const { t } = useTranslation("school");
  const queryClient = useQueryClient();
  const [title, setTitle] = useState(assignment?.title ?? "");
  const [subjectId, setSubjectId] = useState(assignment?.subject?.id ?? "");
  // A new assignment is due at the end of the account's tomorrow.
  const zone = useAccountTimeZone();
  const today = useAccountToday();
  const [due, setDue] = useState(() =>
    toLocalInput(
      toDate(assignment?.dueAt) ?? new Date(zonedMoment(addDays(today, 1), 23 * 60 + 59, zone)),
      zone,
    ),
  );
  const [memo, setMemo] = useState(assignment?.memo ?? "");
  const [remind, setRemind] = useState(assignment?.remind ?? true);
  const [deleting, setDeleting] = useState(false);
  const dueDate = fromLocalInput(due, zone);

  const done = () => {
    refreshSchool(queryClient, "assignments");
    onClose();
  };
  const save = useMutation({
    mutationFn: async () => {
      const dueAt = dueDate ? fromDate(dueDate) : undefined;
      if (assignment)
        await assignmentClient().updateAssignment({
          assignmentId: assignment.id,
          title,
          subjectId,
          dueAt,
          memo,
          remind,
        });
      else await assignmentClient().createAssignment({ title, subjectId, dueAt, memo, remind });
    },
    onSuccess: () => {
      if (!assignment)
        toast.success(remind ? t("assignments.addedWithReminder") : t("assignments.added"));
      done();
    },
    onError: toastError,
  });
  const remove = useMutation({
    mutationFn: () => assignmentClient().deleteAssignment({ assignmentId: assignment?.id ?? "" }),
    onSuccess: done,
    onError: toastError,
  });

  return (
    <>
      <Dialog
        open
        onOpenChange={(open) => !open && onClose()}
        title={assignment ? t("assignments.edit") : t("assignments.add")}
        footer={
          <>
            {assignment && (
              <Button
                variant="ghost"
                className="mr-auto text-danger"
                icon={<Trash2 className="size-4" />}
                onClick={() => setDeleting(true)}
              >
                {t("common:actions.delete")}
              </Button>
            )}
            <Button variant="ghost" onClick={onClose}>
              {t("common:actions.cancel")}
            </Button>
            <Button
              loading={save.isPending}
              disabled={!title.trim() || !dueDate}
              onClick={() => save.mutate()}
            >
              {assignment ? t("common:actions.save") : t("assignments.addAction")}
            </Button>
          </>
        }
      >
        <div className="flex flex-col gap-4">
          <TextField
            label={t("assignments.titleField")}
            value={title}
            maxLength={80}
            placeholder={t("assignments.titlePlaceholder")}
            onChange={(event) => setTitle(event.target.value)}
            autoFocus
          />
          <SubjectSelect value={subjectId} onChange={setSubjectId} />
          <TextField
            label={t("assignments.due")}
            type="datetime-local"
            value={due}
            onChange={(event) => setDue(event.target.value)}
          />
          <TextArea
            label={t("assignments.memo")}
            value={memo}
            maxLength={500}
            onChange={(event) => setMemo(event.target.value)}
          />
          <div className="-mx-3">
            <SwitchRow
              label={t("assignments.remind")}
              hint={t("assignments.remindHint")}
              checked={remind}
              onChange={setRemind}
            />
          </div>
        </div>
      </Dialog>
      <ConfirmDialog
        open={deleting}
        onOpenChange={setDeleting}
        title={t("assignments.deleteTitle")}
        description={assignment?.title}
        confirmLabel={t("common:actions.delete")}
        danger
        loading={remove.isPending}
        onConfirm={() => remove.mutate()}
      />
    </>
  );
}

export function AssignmentsEmpty({ action }: { action?: React.ReactNode }) {
  const { t } = useTranslation("school");
  return (
    <EmptyState
      emoji="🎒"
      title={t("assignments.empty")}
      body={t("assignments.emptyBody")}
      action={action}
    />
  );
}
