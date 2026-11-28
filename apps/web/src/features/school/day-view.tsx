import { useMutation, useQueryClient } from "@tanstack/react-query";
import { school, toMillis } from "@tokuly/api";
import {
  classColor,
  classStatusKey,
  clock,
  dayLabel,
  inkOn,
  refreshSchool,
  takesPlace,
} from "@tokuly/core/school";
import { Undo2 } from "lucide-react";
import { useState } from "react";
import { Button } from "#/components/ui/button";
import { Chip } from "#/components/ui/card";
import { Dialog } from "#/components/ui/dialog";
import { TextField } from "#/components/ui/field";
import { SelectField } from "#/components/ui/select";
import { cn } from "#/lib/cn";
import { useTranslation } from "#/lib/use-translation";
import { timetableClient, toastError } from "./api";

import { Segmented } from "./parts";
import { SubjectSelect } from "./subject-select";

/**
 * One resolved day: every period of the term with the class that takes
 * place in it (or the free slot), and what changed for the date. On the
 * caller's own days a period can be tapped to record a one-off change.
 */
export function DayView({
  day,
  editable,
  nowMs,
}: {
  day: school.TimetableDay;
  editable?: boolean /** Marks the class in progress. */;
  nowMs?: number;
}) {
  const { t, i18n } = useTranslation("school");
  const [changing, setChanging] = useState<number | undefined>(undefined);

  if (day.kind !== school.DayKind.SCHOOL_DAY) {
    const emoji =
      day.kind === school.DayKind.HOLIDAY
        ? "🎌"
        : day.kind === school.DayKind.OUT_OF_TERM
          ? "🏝️"
          : "🛋️";
    const title =
      day.kind === school.DayKind.HOLIDAY
        ? day.label || t("day.holiday")
        : day.kind === school.DayKind.OUT_OF_TERM
          ? t("day.outOfTerm")
          : t("day.dayOff");
    return (
      <div className="flex items-center gap-3 rounded-card bg-surface-2 px-4 py-5">
        <span className="text-3xl">{emoji}</span>
        <span>
          <span className="block font-black">{title}</span>
          <span className="block text-sm text-ink-soft">{t("day.noClasses")}</span>
        </span>
      </div>
    );
  }

  return (
    <>
      {(day.label ||
        day.weekPattern === school.WeekPattern.A ||
        day.weekPattern === school.WeekPattern.B) && (
        <p className="mb-2 flex flex-wrap gap-1.5 px-1">
          {day.label && <Chip>{day.label}</Chip>}
          {day.weekPattern === school.WeekPattern.A && <Chip>{t("timetable.weekA")}</Chip>}
          {day.weekPattern === school.WeekPattern.B && <Chip>{t("timetable.weekB")}</Chip>}
        </p>
      )}
      <ol
        className="flex flex-col gap-1.5"
        aria-label={t("day.classesOf", { date: dayLabel(day.date, i18n.language) })}
      >
        {day.periods.map((period) => {
          const classes = day.classes.filter((item) => item.period === period.number);
          const live =
            nowMs !== undefined &&
            classes.some(
              (item) =>
                takesPlace(item) &&
                toMillis(item.startsAt) <= nowMs &&
                nowMs < toMillis(item.endsAt),
            );
          const body = (
            <>
              <span className="flex w-11 shrink-0 flex-col items-center leading-tight">
                <span className="font-black">
                  {period.label || t("day.period", { period: period.number })}
                </span>
                <span className="text-[11px] font-bold text-ink-faint">
                  {clock(period.startMinute)}
                </span>
              </span>
              <span className="flex min-w-0 flex-1 flex-col gap-1">
                {classes.length === 0 ? (
                  <span className="text-sm font-bold text-ink-faint">{t("day.free")}</span>
                ) : (
                  classes.map((item) => (
                    <ClassLine
                      key={`${item.entryId}-${item.changeId}-${item.status}`}
                      item={item}
                    />
                  ))
                )}
              </span>
              {live && (
                <span className="shrink-0 rounded-full bg-live px-2 py-0.5 text-[11px] font-black text-white">
                  {t("day.now")}
                </span>
              )}
            </>
          );
          const classes_ = cn(
            "flex w-full items-center gap-3 rounded-2xl px-3 py-2.5 text-left",
            live ? "sticker-sm bg-surface" : "bg-surface-2",
          );
          return (
            <li key={period.number}>
              {editable && day.termId ? (
                <button
                  type="button"
                  onClick={() => setChanging(period.number)}
                  className={cn(classes_, "transition-colors hover:bg-surface-3")}
                  aria-label={t("day.changePeriod", { period: period.number })}
                >
                  {body}
                </button>
              ) : (
                <div className={classes_}>{body}</div>
              )}
            </li>
          );
        })}
      </ol>
      {changing !== undefined && (
        <ChangeDialog day={day} period={changing} onClose={() => setChanging(undefined)} />
      )}
    </>
  );
}

function ClassLine({ item }: { item: school.ScheduledClass }) {
  const { t, i18n } = useTranslation("school");
  const off = !takesPlace(item);
  const key = classStatusKey(item.status);
  const color = classColor(item);
  const where =
    item.status === school.ClassStatus.MOVED_AWAY || item.status === school.ClassStatus.MOVED_HERE
      ? t(item.status === school.ClassStatus.MOVED_AWAY ? "day.movedTo" : "day.movedFrom", {
          date: dayLabel(item.otherDate, i18n.language),
          period: item.otherPeriod,
        })
      : undefined;
  return (
    <span className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-0.5">
      <span
        className={cn(
          "inline-flex h-7 max-w-full items-center gap-1 rounded-full border-2 border-ink px-2.5 text-sm font-black",
          off && "opacity-50",
        )}
        style={{ background: color, color: inkOn(color) }}
      >
        {item.subject?.emoji && <span aria-hidden>{item.subject.emoji}</span>}
        <span className={cn("truncate", off && "line-through")}>{item.subject?.name}</span>
      </span>
      {key && (
        <span
          className={cn(
            "rounded-full px-2 py-0.5 text-[11px] font-black",
            off ? "bg-danger text-white" : "bg-sun text-[#17122b]",
          )}
        >
          {t(`day.status.${key}`)}
        </span>
      )}
      <span className="min-w-0 truncate text-xs font-bold text-ink-soft">
        {[where, item.room, item.teacher, item.memo].filter(Boolean).join(" · ")}
      </span>
    </span>
  );
}

/** Records a one-off change for one period of one date, or takes one back. */
function ChangeDialog({
  day,
  period,
  onClose,
}: {
  day: school.TimetableDay;
  period: number;
  onClose(): void;
}) {
  const { t, i18n } = useTranslation("school");
  const queryClient = useQueryClient();
  const classes = day.classes.filter((item) => item.period === period);
  const changes = classes.filter((item) => item.changeId);
  const hasClass = classes.some(
    (item) => item.entryId && item.status !== school.ClassStatus.MOVED_HERE,
  );
  const [kind, setKind] = useState<school.ChangeKind>(
    hasClass ? school.ChangeKind.CANCELLED : school.ChangeKind.EXTRA,
  );
  const [subjectId, setSubjectId] = useState("");
  const [teacher, setTeacher] = useState("");
  const [room, setRoom] = useState("");
  const [memo, setMemo] = useState("");
  const [toDate, setToDate] = useState(day.date);
  const [toPeriod, setToPeriod] = useState(String(period));

  const done = () => {
    refreshSchool(queryClient, "timetable");
    onClose();
  };
  const create = useMutation({
    mutationFn: () =>
      timetableClient().createTimetableChange({
        termId: day.termId,
        date: day.date,
        period,
        kind,
        subjectId:
          kind === school.ChangeKind.REPLACED || kind === school.ChangeKind.EXTRA ? subjectId : "",
        teacher:
          kind === school.ChangeKind.REPLACED || kind === school.ChangeKind.EXTRA ? teacher : "",
        room: kind === school.ChangeKind.REPLACED || kind === school.ChangeKind.EXTRA ? room : "",
        memo,
        toDate: kind === school.ChangeKind.MOVED ? toDate : "",
        toPeriod: kind === school.ChangeKind.MOVED ? Number(toPeriod) : 0,
      }),
    onSuccess: done,
    onError: toastError,
  });
  const undo = useMutation({
    mutationFn: (changeId: string) => timetableClient().deleteTimetableChange({ changeId }),
    onSuccess: done,
    onError: toastError,
  });

  const kinds = hasClass
    ? [
        { value: school.ChangeKind.CANCELLED, label: t("change.cancelled") },
        { value: school.ChangeKind.MOVED, label: t("change.moved") },
        { value: school.ChangeKind.REPLACED, label: t("change.replaced") },
      ]
    : [{ value: school.ChangeKind.EXTRA, label: t("change.extra") }];
  const valid =
    kind === school.ChangeKind.EXTRA
      ? Boolean(subjectId)
      : kind === school.ChangeKind.MOVED
        ? Boolean(toDate) && (toDate !== day.date || Number(toPeriod) !== period)
        : true;

  return (
    <Dialog
      open
      onOpenChange={(open) => !open && onClose()}
      title={t("change.title", { date: dayLabel(day.date, i18n.language), period })}
      description={t("change.lead")}
      footer={
        <>
          <Button variant="ghost" onClick={onClose}>
            {t("common:actions.cancel")}
          </Button>
          <Button loading={create.isPending} disabled={!valid} onClick={() => create.mutate()}>
            {t("change.save")}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-4">
        {changes.map((item) => (
          <div
            key={item.changeId}
            className="flex items-center gap-3 rounded-2xl bg-surface-2 px-3 py-2"
          >
            <span className="min-w-0 flex-1 truncate text-sm font-bold">
              {t(`day.status.${classStatusKey(item.status) ?? "replaced"}`)} · {item.subject?.name}
            </span>
            <Button
              size="sm"
              variant="secondary"
              icon={<Undo2 className="size-4" />}
              loading={undo.isPending}
              onClick={() => undo.mutate(item.changeId)}
            >
              {t("change.undo")}
            </Button>
          </div>
        ))}
        {kinds.length > 1 && (
          <Segmented
            label={t("change.kind")}
            className="w-full"
            value={kind}
            onChange={(next: school.ChangeKind) => setKind(next)}
            options={kinds}
          />
        )}
        {kind === school.ChangeKind.CANCELLED && (
          <p className="px-1 text-sm text-ink-soft">{t("change.cancelledHint")}</p>
        )}
        {kind === school.ChangeKind.MOVED && (
          <div className="grid grid-cols-2 gap-3">
            <TextField
              label={t("change.toDate")}
              type="date"
              value={toDate}
              onChange={(event) => setToDate(event.target.value)}
            />
            <SelectField
              label={t("change.toPeriod")}
              value={toPeriod}
              onChange={setToPeriod}
              options={day.periods.map((item) => ({
                value: String(item.number),
                label: t("day.period", { period: item.number }),
              }))}
            />
          </div>
        )}
        {(kind === school.ChangeKind.REPLACED || kind === school.ChangeKind.EXTRA) && (
          <>
            <SubjectSelect
              label={
                kind === school.ChangeKind.REPLACED
                  ? t("change.replacedSubject")
                  : t("subjects.one")
              }
              value={subjectId}
              onChange={setSubjectId}
            />
            <div className="grid grid-cols-2 gap-3">
              <TextField
                label={t("subjects.teacher")}
                value={teacher}
                maxLength={40}
                onChange={(event) => setTeacher(event.target.value)}
              />
              <TextField
                label={t("subjects.room")}
                value={room}
                maxLength={40}
                onChange={(event) => setRoom(event.target.value)}
              />
            </div>
          </>
        )}
        <TextField
          label={t("timetable.memo")}
          value={memo}
          maxLength={200}
          onChange={(event) => setMemo(event.target.value)}
        />
      </div>
    </Dialog>
  );
}
