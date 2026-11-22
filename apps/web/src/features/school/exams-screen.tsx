import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { school } from "@tokuly/api";
import { ChevronRight, Pencil, Plus, Trash2 } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Chip } from "#/components/ui/card";
import { ConfirmDialog } from "#/components/ui/confirm";
import { Dialog } from "#/components/ui/dialog";
import { TextArea, TextField } from "#/components/ui/field";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { SwitchRow } from "#/components/ui/switch";
import { Tabs } from "#/components/ui/tabs";
import { useAccountToday } from "#/lib/account-time";
import { cn } from "#/lib/cn";
import { examClient, keys, refresh, toastError, useAssignments, useExams } from "./api";
import { addDays, dayLabel, isYmd } from "./dates";
import { AssignmentDialog, AssignmentRow, AssignmentsEmpty, DueList, useExamCountdown } from "./due";
import { Block, LoadFailed, RowsSkeleton, SchoolPage, SubjectPill } from "./parts";
import { SubjectSelect } from "./subject-select";

/** Exams and assignments together: what is due, the exam periods, and the way to the assignment list. */
export function ExamsScreen() {
  const { t, i18n } = useTranslation("school");
  const exams = useExams();
  const countdown = useExamCountdown();
  const [creating, setCreating] = useState(false);
  const [adding, setAdding] = useState(false);
  const list = exams.data ?? [];

  return (
    <SchoolPage section="exams">
      <Block
        title={t("due.title")}
        action={
          <Button size="sm" variant="secondary" icon={<Plus className="size-4" />} onClick={() => setAdding(true)}>
            {t("assignments.add")}
          </Button>
        }
      >
        <DueList />
        <Link to="/school/assignments" className="mt-2 flex items-center justify-end gap-0.5 px-1 text-sm font-bold text-brand">
          {t("assignments.all")}
          <ChevronRight className="size-4" />
        </Link>
      </Block>

      <Block
        title={t("exams.title")}
        action={
          <Button size="sm" icon={<Plus className="size-4" />} onClick={() => setCreating(true)}>
            {t("exams.add")}
          </Button>
        }
      >
        {exams.isPending ? (
          <RowsSkeleton className="h-24" rows={2} />
        ) : exams.isError ? (
          <LoadFailed />
        ) : list.length === 0 ? (
          <EmptyState emoji="📝" title={t("exams.empty")} body={t("exams.emptyBody")} />
        ) : (
          <ul className="flex flex-col gap-3">
            {list.map((exam) => {
              const finished = exam.status === school.ExamStatus.FINISHED;
              return (
                <li key={exam.id}>
                  <Link to="/school/exams/$examId" params={{ examId: exam.id }} className={cn("pressable sticker flex items-center gap-4 rounded-card p-4", finished ? "bg-surface-2" : exam.status === school.ExamStatus.ONGOING ? "bg-sun text-[#17122b]" : "bg-surface")}>
                    <span className="min-w-0 flex-1">
                      <span className="block truncate text-lg font-black">{exam.name}</span>
                      <span className="block text-sm font-bold opacity-70">
                        {dayLabel(exam.startDate, i18n.language)}
                        {exam.endDate !== exam.startDate && ` 〜 ${dayLabel(exam.endDate, i18n.language)}`}
                        {" · "}
                        {t("exams.subjectCount", { count: exam.subjects.length })}
                      </span>
                    </span>
                    <span className="shrink-0 text-right">
                      {exam.status === school.ExamStatus.UPCOMING && exam.daysUntilStart > 0 ? (
                        <>
                          <span className="block text-xs font-black opacity-70">{t("exams.until")}</span>
                          <span className="block text-3xl font-black leading-none">
                            {exam.daysUntilStart}
                            <span className="text-base">{t("exams.days")}</span>
                          </span>
                        </>
                      ) : (
                        <span className="text-sm font-black">{countdown(exam)}</span>
                      )}
                    </span>
                  </Link>
                </li>
              );
            })}
          </ul>
        )}
      </Block>
      {creating && <ExamDialog onClose={() => setCreating(false)} />}
      {adding && <AssignmentDialog onClose={() => setAdding(false)} />}
    </SchoolPage>
  );
}

/** Creates an exam period or changes one. */
function ExamDialog({ exam, onClose }: { exam?: school.Exam; onClose(): void }) {
  const { t } = useTranslation("school");
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const [name, setName] = useState(exam?.name ?? "");
  const today = useAccountToday();
  const [startDate, setStartDate] = useState(exam?.startDate ?? addDays(today, 14));
  const [endDate, setEndDate] = useState(exam?.endDate ?? "");
  const [showOnProfile, setShowOnProfile] = useState(exam?.showOnProfile ?? false);
  const [memo, setMemo] = useState(exam?.memo ?? "");
  const save = useMutation({
    mutationFn: async () => {
      const input = { name, startDate, endDate: endDate || startDate, showOnProfile, memo };
      return exam ? (await examClient().updateExam({ examId: exam.id, ...input })).exam : (await examClient().createExam(input)).exam;
    },
    onSuccess: (saved) => {
      refresh(queryClient, "exams");
      onClose();
      if (!exam && saved) void navigate({ to: "/school/exams/$examId", params: { examId: saved.id } });
    },
    onError: toastError,
  });
  return (
    <Dialog
      open
      onOpenChange={(open) => !open && onClose()}
      title={exam ? t("exams.edit") : t("exams.add")}
      footer={
        <>
          <Button variant="ghost" onClick={onClose}>
            {t("common:actions.cancel")}
          </Button>
          <Button loading={save.isPending} disabled={!name.trim() || !isYmd(startDate) || (endDate !== "" && !isYmd(endDate))} onClick={() => save.mutate()}>
            {exam ? t("common:actions.save") : t("exams.addAction")}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-4">
        <TextField label={t("exams.name")} value={name} maxLength={40} placeholder={t("exams.namePlaceholder")} onChange={(event) => setName(event.target.value)} autoFocus />
        <div className="grid grid-cols-2 gap-3">
          <TextField label={t("exams.start")} type="date" value={startDate} onChange={(event) => setStartDate(event.target.value)} />
          <TextField label={t("exams.end")} type="date" value={endDate} min={startDate} onChange={(event) => setEndDate(event.target.value)} />
        </div>
        <TextArea label={t("exams.memo")} value={memo} maxLength={500} onChange={(event) => setMemo(event.target.value)} />
        <div className="-mx-3">
          <SwitchRow label={t("exams.showOnProfile")} hint={t("exams.showOnProfileHint")} checked={showOnProfile} onChange={setShowOnProfile} />
        </div>
      </div>
    </Dialog>
  );
}

/** One exam period: the countdown, each subject's day and scope, and how it went. */
export function ExamScreen({ examId }: { examId: string }) {
  const { t, i18n } = useTranslation("school");
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const countdown = useExamCountdown();
  const { data: exam, isPending, isError } = useQuery({ queryKey: keys.exam(examId), queryFn: async () => (await examClient().getExam({ examId })).exam });
  const [editing, setEditing] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [subject, setSubject] = useState<school.ExamSubject | "new" | undefined>(undefined);
  const [result, setResult] = useState<school.ExamSubject | undefined>(undefined);

  const remove = useMutation({
    mutationFn: () => examClient().deleteExam({ examId }),
    onSuccess: () => {
      toast.success(t("exams.deleted"));
      refresh(queryClient, "exams");
      void navigate({ to: "/school/exams" });
    },
    onError: toastError,
  });
  const profile = useMutation({
    mutationFn: (showOnProfile: boolean) => examClient().updateExam({ examId, showOnProfile }),
    onSuccess: () => refresh(queryClient, "exams"),
    onError: toastError,
  });

  if (isPending) {
    return (
      <SchoolPage back="/school/exams" title={t("exams.one")}>
        <Skeleton className="h-40" />
      </SchoolPage>
    );
  }
  if (isError || !exam) {
    return (
      <SchoolPage back="/school/exams" title={t("exams.one")}>
        <EmptyState emoji="🫥" title={t("errors.EXAM_NOT_FOUND")} />
      </SchoolPage>
    );
  }
  const ongoing = exam.status === school.ExamStatus.ONGOING;
  const finished = exam.status === school.ExamStatus.FINISHED;

  return (
    <SchoolPage
      back="/school/exams"
      title={exam.name}
      actions={
        <>
          <Button size="icon-sm" variant="ghost" aria-label={t("exams.edit")} onClick={() => setEditing(true)}>
            <Pencil className="size-4" />
          </Button>
          <Button size="icon-sm" variant="ghost" aria-label={t("exams.delete")} onClick={() => setDeleting(true)}>
            <Trash2 className="size-4" />
          </Button>
        </>
      }
    >
      <div className={cn("sticker rounded-card p-5 text-center", ongoing ? "bg-sun text-[#17122b]" : finished ? "bg-surface-2" : "bg-lime text-[#17122b]")}>
        <p className="text-sm font-black opacity-70">
          {dayLabel(exam.startDate, i18n.language)}
          {exam.endDate !== exam.startDate && ` 〜 ${dayLabel(exam.endDate, i18n.language)}`}
        </p>
        {exam.status === school.ExamStatus.UPCOMING && exam.daysUntilStart > 0 ? (
          <p className="mt-1 font-black">
            <span className="text-base">{t("exams.until")}</span>
            <span className="mx-1 text-6xl leading-none">{exam.daysUntilStart}</span>
            <span className="text-base">{t("exams.days")}</span>
          </p>
        ) : (
          <p className="mt-2 text-3xl font-black">{countdown(exam)}</p>
        )}
        {exam.memo && <p className="mt-2 text-sm font-bold whitespace-pre-wrap opacity-80">{exam.memo}</p>}
      </div>
      <div className="-mx-1 mt-2">
        <SwitchRow label={t("exams.showOnProfile")} hint={t("exams.showOnProfileHint")} checked={exam.showOnProfile} onChange={(checked) => profile.mutate(checked)} />
      </div>

      <Block
        title={t("exams.subjects")}
        action={
          <Button size="sm" icon={<Plus className="size-4" />} onClick={() => setSubject("new")}>
            {t("exams.addSubject")}
          </Button>
        }
      >
        {exam.subjects.length === 0 ? (
          <EmptyState emoji="✏️" title={t("exams.noSubjects")} body={t("exams.noSubjectsBody")} />
        ) : (
          <ul className="flex flex-col gap-2">
            {exam.subjects.map((item) => (
              <li key={item.id} className="rounded-card bg-surface-2 p-3">
                <div className="flex items-center gap-2">
                  <SubjectPill subject={item.subject} className="h-7 px-2.5 text-sm" />
                  <span className="min-w-0 flex-1 truncate text-sm font-bold text-ink-soft">
                    {item.date ? dayLabel(item.date, i18n.language) : t("exams.dateUnknown")}
                    {item.period > 0 && ` · ${t("day.period", { period: item.period })}`}
                  </span>
                  <Button size="icon-sm" variant="ghost" aria-label={t("exams.editSubject", { name: item.subject?.name ?? "" })} onClick={() => setSubject(item)}>
                    <Pencil className="size-4" />
                  </Button>
                </div>
                {item.scope && <p className="mt-2 px-1 text-sm whitespace-pre-wrap">{item.scope}</p>}
                <div className="mt-2 flex items-center gap-2 px-1">
                  {item.result ? (
                    <span className="flex min-w-0 flex-1 flex-wrap items-center gap-1.5">
                      {item.result.score !== undefined && (
                        <span className="text-lg font-black">
                          {item.result.score}
                          <span className="text-sm text-ink-soft"> / {item.result.maxScore || 100}</span>
                        </span>
                      )}
                      {item.result.average !== undefined && <Chip>{t("exams.result.averageValue", { value: item.result.average })}</Chip>}
                      {item.result.rank !== undefined && <Chip>{t("exams.result.rankValue", { value: item.result.rank })}</Chip>}
                      {item.result.memo && <span className="min-w-0 truncate text-sm text-ink-soft">{item.result.memo}</span>}
                    </span>
                  ) : (
                    <span className="min-w-0 flex-1 text-sm text-ink-faint">{t("exams.result.none")}</span>
                  )}
                  <Button size="sm" variant="secondary" onClick={() => setResult(item)}>
                    {item.result ? t("exams.result.edit") : t("exams.result.add")}
                  </Button>
                </div>
              </li>
            ))}
          </ul>
        )}
      </Block>

      {editing && <ExamDialog exam={exam} onClose={() => setEditing(false)} />}
      {subject && <ExamSubjectDialog exam={exam} item={subject === "new" ? undefined : subject} onClose={() => setSubject(undefined)} />}
      {result && <ResultDialog item={result} onClose={() => setResult(undefined)} />}
      <ConfirmDialog
        open={deleting}
        onOpenChange={setDeleting}
        title={t("exams.deleteTitle", { name: exam.name })}
        description={t("exams.deleteBody")}
        confirmLabel={t("common:actions.delete")}
        danger
        loading={remove.isPending}
        onConfirm={() => remove.mutate()}
      />
    </SchoolPage>
  );
}

function ExamSubjectDialog({ exam, item, onClose }: { exam: school.Exam; item?: school.ExamSubject; onClose(): void }) {
  const { t } = useTranslation("school");
  const queryClient = useQueryClient();
  const [subjectId, setSubjectId] = useState(item?.subject?.id ?? "");
  const [date, setDate] = useState(item?.date ?? "");
  const [period, setPeriod] = useState(item?.period ? String(item.period) : "");
  const [scope, setScope] = useState(item?.scope ?? "");
  const done = () => {
    refresh(queryClient, "exams");
    onClose();
  };
  const save = useMutation({
    mutationFn: () => examClient().putExamSubject({ examId: exam.id, examSubjectId: item?.id ?? "", subjectId, date, period: Number(period) || 0, scope }),
    onSuccess: done,
    onError: toastError,
  });
  const remove = useMutation({
    mutationFn: () => examClient().deleteExamSubject({ examSubjectId: item?.id ?? "" }),
    onSuccess: done,
    onError: toastError,
  });
  return (
    <Dialog
      open
      onOpenChange={(open) => !open && onClose()}
      title={item ? t("exams.editSubject", { name: item.subject?.name ?? "" }) : t("exams.addSubject")}
      footer={
        <>
          {item && (
            <Button variant="ghost" className="mr-auto text-danger" icon={<Trash2 className="size-4" />} loading={remove.isPending} onClick={() => remove.mutate()}>
              {t("exams.removeSubject")}
            </Button>
          )}
          <Button loading={save.isPending} disabled={!subjectId} onClick={() => save.mutate()}>
            {t("common:actions.save")}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-4">
        <SubjectSelect value={subjectId} onChange={setSubjectId} />
        <div className="grid grid-cols-2 gap-3">
          <TextField label={t("exams.subjectDate")} type="date" value={date} min={exam.startDate} max={exam.endDate} onChange={(event) => setDate(event.target.value)} />
          <TextField label={t("exams.subjectPeriod")} type="number" inputMode="numeric" min={1} max={12} value={period} onChange={(event) => setPeriod(event.target.value)} />
        </div>
        <TextArea label={t("exams.scope")} value={scope} maxLength={1000} placeholder={t("exams.scopePlaceholder")} onChange={(event) => setScope(event.target.value)} />
      </div>
    </Dialog>
  );
}

const numberOrUndefined = (value: string) => (value.trim() === "" || Number.isNaN(Number(value)) ? undefined : Number(value));

/** The private note of how one test went: score, average, rank. */
function ResultDialog({ item, onClose }: { item: school.ExamSubject; onClose(): void }) {
  const { t } = useTranslation("school");
  const queryClient = useQueryClient();
  const [score, setScore] = useState(item.result?.score?.toString() ?? "");
  const [maxScore, setMaxScore] = useState(String(item.result?.maxScore || 100));
  const [average, setAverage] = useState(item.result?.average?.toString() ?? "");
  const [rank, setRank] = useState(item.result?.rank?.toString() ?? "");
  const [memo, setMemo] = useState(item.result?.memo ?? "");
  const save = useMutation({
    mutationFn: (clear: boolean) =>
      examClient().setExamResult({
        examSubjectId: item.id,
        result: clear ? undefined : { score: numberOrUndefined(score), maxScore: Number(maxScore) || 100, average: numberOrUndefined(average), rank: numberOrUndefined(rank), memo },
      }),
    onSuccess: () => {
      refresh(queryClient, "exams");
      onClose();
    },
    onError: toastError,
  });
  return (
    <Dialog
      open
      onOpenChange={(open) => !open && onClose()}
      title={t("exams.result.title", { name: item.subject?.name ?? "" })}
      description={t("exams.result.lead")}
      footer={
        <>
          {item.result && (
            <Button variant="ghost" className="mr-auto text-danger" onClick={() => save.mutate(true)}>
              {t("exams.result.clear")}
            </Button>
          )}
          <Button loading={save.isPending} onClick={() => save.mutate(false)}>
            {t("common:actions.save")}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-4">
        <div className="grid grid-cols-2 gap-3">
          <TextField label={t("exams.result.score")} type="number" inputMode="numeric" value={score} onChange={(event) => setScore(event.target.value)} autoFocus />
          <TextField label={t("exams.result.maxScore")} type="number" inputMode="numeric" value={maxScore} onChange={(event) => setMaxScore(event.target.value)} />
          <TextField label={t("exams.result.average")} type="number" inputMode="decimal" value={average} onChange={(event) => setAverage(event.target.value)} />
          <TextField label={t("exams.result.rank")} type="number" inputMode="numeric" value={rank} onChange={(event) => setRank(event.target.value)} />
        </div>
        <TextField label={t("exams.result.memo")} value={memo} maxLength={200} onChange={(event) => setMemo(event.target.value)} />
      </div>
    </Dialog>
  );
}

type AssignmentTab = "open" | "done" | "all";
const FILTERS: Record<AssignmentTab, school.AssignmentFilter> = { open: school.AssignmentFilter.OPEN, done: school.AssignmentFilter.DONE, all: school.AssignmentFilter.ALL };

/** Every assignment: the open ones by urgency, the finished ones, adding and changing them. */
export function AssignmentsScreen() {
  const { t } = useTranslation("school");
  const [tab, setTab] = useState<AssignmentTab>("open");
  const [opened, setOpened] = useState<school.Assignment | "new" | undefined>(undefined);
  const assignments = useAssignments(FILTERS[tab]);
  const list = assignments.data?.assignments ?? [];
  const add = (
    <Button size="sm" icon={<Plus className="size-4" />} onClick={() => setOpened("new")}>
      {t("assignments.add")}
    </Button>
  );
  return (
    <SchoolPage back="/school/exams" title={t("assignments.title")} actions={add}>
      <div className="mb-4 flex flex-wrap items-center gap-2">
        <Tabs
          value={tab}
          onChange={(next: AssignmentTab) => setTab(next)}
          items={[
            { value: "open", label: t("assignments.tabs.open"), badge: tab === "open" ? undefined : assignments.data?.overdueCount },
            { value: "done", label: t("assignments.tabs.done") },
            { value: "all", label: t("assignments.tabs.all") },
          ]}
        />
        {assignments.data && tab === "open" && assignments.data.openCount > 0 && (
          <p className="text-sm font-bold text-ink-soft">
            {t("assignments.openCount", { count: assignments.data.openCount })}
            {assignments.data.overdueCount > 0 && <span className="text-danger"> · {t("assignments.overdueCount", { count: assignments.data.overdueCount })}</span>}
          </p>
        )}
      </div>
      {assignments.isPending ? (
        <RowsSkeleton />
      ) : assignments.isError ? (
        <LoadFailed />
      ) : list.length === 0 ? (
        tab === "open" ? (
          <AssignmentsEmpty action={add} />
        ) : (
          <EmptyState emoji="🌱" title={t("assignments.emptyDone")} />
        )
      ) : (
        <ul className="flex flex-col gap-1.5">
          {list.map((assignment) => (
            <AssignmentRow key={assignment.id} assignment={assignment} onOpen={setOpened} />
          ))}
        </ul>
      )}
      {opened && <AssignmentDialog assignment={opened === "new" ? undefined : opened} onClose={() => setOpened(undefined)} />}
    </SchoolPage>
  );
}
