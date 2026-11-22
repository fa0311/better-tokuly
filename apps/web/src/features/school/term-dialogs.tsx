import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import type { TFunction } from "i18next";
import { school } from "@tokuly/api";
import { Plus, Trash2, X } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { Dialog } from "#/components/ui/dialog";
import { TextField } from "#/components/ui/field";
import { SelectField } from "#/components/ui/select";
import { SwitchRow } from "#/components/ui/switch";
import { cn } from "#/lib/cn";
import { keys, refresh, timetableClient, toastError } from "./api";
import { addDays, clockInput, dayLabel, isYmd, layoutWeekdays, parseClock, WEEKDAYS, ymd } from "./dates";
import { FriendPicker, Segmented } from "./parts";

interface PeriodDraft {
  start: string;
  end: string;
}

interface TermDraft {
  name: string;
  startDate: string;
  endDate: string;
  preset: school.LayoutPreset;
  periods: PeriodDraft[];
  weekdays: school.Weekday[];
  alternating: boolean;
  weekADate: string;
  holidays: boolean;
  current: boolean;
}

const toDrafts = (periods: school.Period[]): PeriodDraft[] => periods.map((period) => ({ start: clockInput(period.startMinute), end: clockInput(period.endMinute) }));

/** A term's name for today: 前期 from April, 後期 from October. */
function defaultTerm(t: TFunction<"school">): Pick<TermDraft, "name" | "startDate" | "endDate"> {
  const now = new Date();
  const month = now.getMonth() + 1;
  const first = month >= 4 && month <= 9;
  const schoolYear = month >= 4 ? now.getFullYear() : now.getFullYear() - 1;
  const end = first ? `${schoolYear}-09-30` : `${schoolYear + 1}-03-31`;
  return { name: t(first ? "terms.defaultFirst" : "terms.defaultSecond", { year: schoolYear }), startDate: ymd(now), endDate: end };
}

/**
 * Creates a term or changes one: its name and dates, the period times (from
 * a preset or entered by hand), the weekdays with classes, A/B weeks, public
 * holidays, and whether it is the term in use.
 */
export function TermDialog({ open, onOpenChange, term, onSaved }: { open: boolean; onOpenChange(open: boolean): void; /** The term to change; a new one when unset. */ term?: school.Term; onSaved?(term: school.Term): void }) {
  const { t, i18n } = useTranslation("school");
  const queryClient = useQueryClient();
  const presets = useQuery({ queryKey: keys.presets, queryFn: async () => (await timetableClient().listLayoutPresets({})).presets, enabled: open });
  const [draft, setDraft] = useState<TermDraft | undefined>(undefined);
  const [deleting, setDeleting] = useState(false);

  // The form starts from the term (or the defaults) each time the dialog opens.
  if (open && !draft) {
    setDraft(
      term
        ? {
            name: term.name,
            startDate: term.startDate,
            endDate: term.endDate,
            preset: term.layout?.preset || school.LayoutPreset.CUSTOM,
            periods: toDrafts(term.layout?.periods ?? []),
            weekdays: layoutWeekdays(term.layout),
            alternating: term.alternatingWeeks,
            weekADate: term.weekADate || term.startDate,
            holidays: term.observePublicHolidays,
            current: term.current,
          }
        : { ...defaultTerm(t), preset: school.LayoutPreset.HIGH_SCHOOL_6, periods: [], weekdays: WEEKDAYS.slice(0, 5), alternating: false, weekADate: ymd(), holidays: true, current: true },
    );
  }
  if (!open && draft) setDraft(undefined);

  // A new term shows the periods of the chosen preset once the presets arrived.
  const presetPeriods = (preset: school.LayoutPreset) => toDrafts(presets.data?.find((info) => info.preset === preset)?.layout?.periods ?? []);
  const periods = draft && draft.periods.length === 0 && draft.preset !== school.LayoutPreset.CUSTOM ? presetPeriods(draft.preset) : (draft?.periods ?? []);

  const save = useMutation({
    mutationFn: async (input: TermDraft) => {
      const layout = {
        preset: input.preset,
        periods: periods.flatMap((period, index) => {
          const startMinute = parseClock(period.start);
          const endMinute = parseClock(period.end);
          return startMinute === undefined || endMinute === undefined ? [] : [{ number: index + 1, startMinute, endMinute }];
        }),
        weekdays: input.weekdays,
      };
      const shared = { name: input.name, startDate: input.startDate, endDate: input.endDate, layout, alternatingWeeks: input.alternating, weekADate: input.alternating ? input.weekADate : "", observePublicHolidays: input.holidays };
      if (term) return timetableClient().updateTerm({ termId: term.id, ...shared, ...(input.current && !term.current ? { current: true } : {}) });
      return { ...(await timetableClient().createTerm({ ...shared, current: input.current })), removedEntryCount: 0 };
    },
    onSuccess: (response) => {
      if (response.removedEntryCount > 0) toast(t("terms.removedEntries", { count: response.removedEntryCount }));
      refresh(queryClient, "timetable");
      if (response.term) onSaved?.(response.term);
      onOpenChange(false);
    },
    onError: toastError,
  });
  const remove = useMutation({
    mutationFn: () => timetableClient().deleteTerm({ termId: term?.id ?? "" }),
    onSuccess: () => {
      toast.success(t("terms.deleted"));
      refresh(queryClient, "timetable");
      setDeleting(false);
      onOpenChange(false);
    },
    onError: toastError,
  });

  if (!draft) return null;
  const update = (patch: Partial<TermDraft>) => setDraft({ ...draft, ...patch });
  const setPeriods = (next: PeriodDraft[]) => update({ periods: next, preset: school.LayoutPreset.CUSTOM });
  const valid = draft.name.trim() && isYmd(draft.startDate) && isYmd(draft.endDate) && periods.length > 0 && draft.weekdays.length > 0 && (!draft.alternating || isYmd(draft.weekADate));
  const presetOptions = [
    ...(presets.data ?? []).map((info) => ({ value: String(info.preset), label: (i18n.language.startsWith("ja") ? info.name?.ja : info.name?.en) || info.name?.ja || "", hint: (i18n.language.startsWith("ja") ? info.description?.ja : info.description?.en) || undefined })),
    { value: String(school.LayoutPreset.CUSTOM), label: t("terms.custom") },
  ];

  return (
    <>
      <Dialog
        open={open}
        onOpenChange={onOpenChange}
        title={term ? t("terms.edit") : t("terms.create")}
        footer={
          <>
            {term && (
              <Button variant="ghost" className="mr-auto text-danger" icon={<Trash2 className="size-4" />} onClick={() => setDeleting(true)}>
                {t("common:actions.delete")}
              </Button>
            )}
            <Button variant="ghost" onClick={() => onOpenChange(false)}>
              {t("common:actions.cancel")}
            </Button>
            <Button loading={save.isPending} disabled={!valid} onClick={() => save.mutate(draft)}>
              {term ? t("common:actions.save") : t("terms.createAction")}
            </Button>
          </>
        }
      >
        <div className="flex flex-col gap-4">
          <TextField label={t("terms.name")} value={draft.name} maxLength={40} onChange={(event) => update({ name: event.target.value })} />
          <div className="grid grid-cols-2 gap-3">
            <TextField label={t("terms.start")} type="date" value={draft.startDate} onChange={(event) => update({ startDate: event.target.value })} />
            <TextField label={t("terms.end")} type="date" value={draft.endDate} onChange={(event) => update({ endDate: event.target.value })} />
          </div>

          <SelectField
            label={t("terms.layout")}
            value={String(draft.preset)}
            onChange={(value) => {
              const preset = Number(value) as school.LayoutPreset;
              update({ preset, periods: preset === school.LayoutPreset.CUSTOM ? periods : presetPeriods(preset) });
            }}
            options={presetOptions}
          />
          <div>
            <p className="px-1 pb-1.5 text-sm font-bold text-ink-soft">{t("terms.periods")}</p>
            <ol className="flex flex-col gap-1.5">
              {periods.map((period, index) => (
                <li key={index} className="flex items-center gap-2">
                  <span className="w-8 text-center text-sm font-black">{index + 1}</span>
                  <input
                    type="time"
                    aria-label={t("terms.periodStart", { period: index + 1 })}
                    value={period.start}
                    onChange={(event) => setPeriods(periods.map((item, at) => (at === index ? { ...item, start: event.target.value } : item)))}
                    className="h-10 min-w-0 flex-1 rounded-control border-2 border-line bg-surface px-3 font-bold focus:border-brand focus:outline-none"
                  />
                  <span className="text-ink-faint">〜</span>
                  <input
                    type="time"
                    aria-label={t("terms.periodEnd", { period: index + 1 })}
                    value={period.end}
                    onChange={(event) => setPeriods(periods.map((item, at) => (at === index ? { ...item, end: event.target.value } : item)))}
                    className="h-10 min-w-0 flex-1 rounded-control border-2 border-line bg-surface px-3 font-bold focus:border-brand focus:outline-none"
                  />
                  <Button size="icon-sm" variant="ghost" aria-label={t("terms.removePeriod", { period: index + 1 })} disabled={periods.length <= 1} onClick={() => setPeriods(periods.filter((_, at) => at !== index))}>
                    <X className="size-4" />
                  </Button>
                </li>
              ))}
            </ol>
            {periods.length < 12 && (
              <Button
                size="sm"
                variant="ghost"
                className="mt-1"
                icon={<Plus className="size-4" />}
                onClick={() => {
                  const last = parseClock(periods.at(-1)?.end ?? "08:40") ?? 520;
                  const length = Math.max(10, (parseClock(periods.at(-1)?.end ?? "") ?? 50) - (parseClock(periods.at(-1)?.start ?? "") ?? 0));
                  const start = Math.min(last + 10, 23 * 60);
                  setPeriods([...periods, { start: clockInput(start), end: clockInput(Math.min(start + length, 24 * 60 - 1)) }]);
                }}
              >
                {t("terms.addPeriod")}
              </Button>
            )}
          </div>

          <div>
            <p className="px-1 pb-1.5 text-sm font-bold text-ink-soft">{t("terms.weekdays")}</p>
            <div className="flex flex-wrap gap-1.5">
              {WEEKDAYS.map((weekday) => {
                const on = draft.weekdays.includes(weekday);
                return (
                  <button
                    key={weekday}
                    type="button"
                    aria-pressed={on}
                    aria-label={t(`weekdays.long.${weekday}`)}
                    onClick={() => update({ weekdays: on ? draft.weekdays.filter((day) => day !== weekday) : [...draft.weekdays, weekday] })}
                    className={cn("size-10 rounded-full border-2 text-sm font-black", on ? "border-ink bg-lime text-[#17122b]" : "border-line text-ink-soft")}
                  >
                    {t(`weekdays.short.${weekday}`)}
                  </button>
                );
              })}
            </div>
          </div>

          <div className="-mx-3 flex flex-col">
            <SwitchRow label={t("terms.alternating")} hint={t("terms.alternatingHint")} checked={draft.alternating} onChange={(alternating) => update({ alternating })} />
            {draft.alternating && (
              <div className="px-3 pb-2">
                <TextField label={t("terms.weekADate")} hint={t("terms.weekADateHint")} type="date" value={draft.weekADate} onChange={(event) => update({ weekADate: event.target.value })} />
              </div>
            )}
            <SwitchRow label={t("terms.holidays")} hint={t("terms.holidaysHint")} checked={draft.holidays} onChange={(holidays) => update({ holidays })} />
            {!(term?.current ?? false) && <SwitchRow label={t("terms.current")} hint={t("terms.currentHint")} checked={draft.current} onChange={(current) => update({ current })} />}
          </div>
        </div>
      </Dialog>
      <ConfirmDialog
        open={deleting}
        onOpenChange={setDeleting}
        title={t("terms.deleteTitle", { name: term?.name ?? "" })}
        description={t("terms.deleteBody")}
        confirmLabel={t("common:actions.delete")}
        danger
        loading={remove.isPending}
        onConfirm={() => remove.mutate()}
      />
    </>
  );
}

/** Who may see a term: nobody, chosen friends, or every friend. */
export function ShareDialog({ open, onOpenChange, term }: { open: boolean; onOpenChange(open: boolean): void; term: school.Term }) {
  const { t } = useTranslation("school");
  const queryClient = useQueryClient();
  const [audience, setAudience] = useState<school.TimetableAudience>(term.audience || school.TimetableAudience.NOBODY);
  const [chosen, setChosen] = useState(() => term.sharedWith.map((user) => user.id));
  const save = useMutation({
    mutationFn: () => timetableClient().updateTerm({ termId: term.id, audience, sharedWith: { userIds: audience === school.TimetableAudience.CHOSEN ? chosen : [] } }),
    onSuccess: () => {
      toast.success(audience === school.TimetableAudience.NOBODY ? t("share.savedPrivate") : t("share.saved"));
      refresh(queryClient, "timetable");
      onOpenChange(false);
    },
    onError: toastError,
  });
  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("share.title")}
      description={t("share.lead", { name: term.name })}
      footer={
        <>
          <Button variant="ghost" onClick={() => onOpenChange(false)}>
            {t("common:actions.cancel")}
          </Button>
          <Button loading={save.isPending} disabled={audience === school.TimetableAudience.CHOSEN && chosen.length === 0} onClick={() => save.mutate()}>
            {t("common:actions.save")}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-4">
        <Segmented
          label={t("share.audience")}
          className="w-full"
          value={audience}
          onChange={(next: school.TimetableAudience) => setAudience(next)}
          options={[
            { value: school.TimetableAudience.NOBODY, label: t("share.nobody") },
            { value: school.TimetableAudience.CHOSEN, label: t("share.chosen") },
            { value: school.TimetableAudience.FRIENDS, label: t("share.friends") },
          ]}
        />
        <p className="px-1 text-sm text-ink-soft">
          {audience === school.TimetableAudience.NOBODY ? t("share.nobodyHint") : audience === school.TimetableAudience.CHOSEN ? t("share.chosenHint") : t("share.friendsHint")}
        </p>
        {audience === school.TimetableAudience.CHOSEN && <FriendPicker selected={chosen} onChange={setChosen} />}
      </div>
    </Dialog>
  );
}

/** Days that differ from the weekly rhythm: holidays, class days (optionally with another weekday's timetable), pinned A/B weeks. */
export function DayOverridesDialog({ open, onOpenChange, term, overrides }: { open: boolean; onOpenChange(open: boolean): void; term: school.Term; overrides: school.DayOverride[] }) {
  const { t, i18n } = useTranslation("school");
  const queryClient = useQueryClient();
  const [kind, setKind] = useState<school.DayOverrideKind>(school.DayOverrideKind.HOLIDAY);
  const [startDate, setStartDate] = useState(() => ymd());
  const [endDate, setEndDate] = useState("");
  const [name, setName] = useState("");
  const [follows, setFollows] = useState<school.Weekday>(school.Weekday.UNSPECIFIED);
  const [pattern, setPattern] = useState<school.WeekPattern>(school.WeekPattern.A);

  const add = useMutation({
    mutationFn: () =>
      timetableClient().createDayOverride({
        termId: term.id,
        startDate,
        endDate,
        kind,
        name,
        followsWeekday: kind === school.DayOverrideKind.CLASS_DAY ? follows : school.Weekday.UNSPECIFIED,
        weekPattern: kind === school.DayOverrideKind.WEEK_PATTERN ? pattern : school.WeekPattern.UNSPECIFIED,
      }),
    onSuccess: () => {
      refresh(queryClient, "timetable");
      setName("");
      setEndDate("");
    },
    onError: toastError,
  });
  const remove = useMutation({
    mutationFn: (dayOverrideId: string) => timetableClient().deleteDayOverride({ dayOverrideId }),
    onSuccess: () => refresh(queryClient, "timetable"),
    onError: toastError,
  });

  const describe = (override: school.DayOverride) => {
    if (override.kind === school.DayOverrideKind.HOLIDAY) return t("overrides.holiday");
    if (override.kind === school.DayOverrideKind.WEEK_PATTERN) return t(override.weekPattern === school.WeekPattern.B ? "overrides.weekB" : "overrides.weekA");
    return override.followsWeekday ? t("overrides.follows", { weekday: t(`weekdays.long.${override.followsWeekday}`) }) : t("overrides.classDay");
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange} title={t("overrides.title")} description={t("overrides.lead")}>
      <div className="flex flex-col gap-5">
        {overrides.length > 0 && (
          <ul className="flex flex-col gap-1.5">
            {overrides.map((override) => (
              <li key={override.id} className="flex items-center gap-3 rounded-2xl bg-surface-2 px-3 py-2">
                <span className="min-w-0 flex-1">
                  <span className="block truncate font-bold">{override.name || describe(override)}</span>
                  <span className="block truncate text-sm text-ink-soft">
                    {dayLabel(override.startDate, i18n.language)}
                    {override.endDate !== override.startDate && ` 〜 ${dayLabel(override.endDate, i18n.language)}`}
                    {override.name && ` · ${describe(override)}`}
                  </span>
                </span>
                <Button size="icon-sm" variant="ghost" aria-label={t("overrides.remove", { name: override.name || describe(override) })} onClick={() => remove.mutate(override.id)}>
                  <Trash2 className="size-4" />
                </Button>
              </li>
            ))}
          </ul>
        )}
        <div className="flex flex-col gap-3 rounded-card border-2 border-line p-4">
          <p className="font-black">{t("overrides.add")}</p>
          <SelectField
            label={t("overrides.kind")}
            value={String(kind)}
            onChange={(value) => setKind(Number(value) as school.DayOverrideKind)}
            options={[
              { value: String(school.DayOverrideKind.HOLIDAY), label: t("overrides.holiday"), hint: t("overrides.holidayHint") },
              { value: String(school.DayOverrideKind.CLASS_DAY), label: t("overrides.classDay"), hint: t("overrides.classDayHint") },
              ...(term.alternatingWeeks ? [{ value: String(school.DayOverrideKind.WEEK_PATTERN), label: t("overrides.weekPattern"), hint: t("overrides.weekPatternHint") }] : []),
            ]}
          />
          <div className="grid grid-cols-2 gap-3">
            <TextField label={t("overrides.from")} type="date" value={startDate} min={term.startDate} max={term.endDate} onChange={(event) => setStartDate(event.target.value)} />
            <TextField label={t("overrides.to")} type="date" value={endDate} min={startDate} max={term.endDate} onChange={(event) => setEndDate(event.target.value)} />
          </div>
          <TextField label={t("overrides.name")} value={name} maxLength={40} placeholder={t(kind === school.DayOverrideKind.HOLIDAY ? "overrides.namePlaceholderHoliday" : "overrides.namePlaceholderClass")} onChange={(event) => setName(event.target.value)} />
          {kind === school.DayOverrideKind.CLASS_DAY && (
            <SelectField
              label={t("overrides.followsLabel")}
              value={String(follows)}
              onChange={(value) => setFollows(Number(value) as school.Weekday)}
              options={[{ value: String(school.Weekday.UNSPECIFIED), label: t("overrides.followsOwn") }, ...layoutWeekdays(term.layout).map((weekday) => ({ value: String(weekday), label: t(`weekdays.long.${weekday}`) }))]}
            />
          )}
          {kind === school.DayOverrideKind.WEEK_PATTERN && (
            <Segmented
              label={t("overrides.weekPattern")}
              value={pattern}
              onChange={(next: school.WeekPattern) => setPattern(next)}
              options={[
                { value: school.WeekPattern.A, label: t("timetable.weekA") },
                { value: school.WeekPattern.B, label: t("timetable.weekB") },
              ]}
            />
          )}
          <Button loading={add.isPending} disabled={!isYmd(startDate) || (endDate !== "" && !isYmd(endDate))} onClick={() => add.mutate()}>
            {t("overrides.addAction")}
          </Button>
        </div>
      </div>
    </Dialog>
  );
}

/** Copies a friend's shared term into the caller's account: as a new term, or into the empty slots of one they have. */
export function CopyDialog({ open, onOpenChange, friend, term, ownTerms }: { open: boolean; onOpenChange(open: boolean): void; friend: { id: string; displayName: string }; term: school.Term; ownTerms: school.Term[] }) {
  const { t } = useTranslation("school");
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const [target, setTarget] = useState("");
  const [name, setName] = useState(term.name);
  const [overwrite, setOverwrite] = useState(false);
  const copy = useMutation({
    mutationFn: () => timetableClient().copyFriendTimetable({ userId: friend.id, termId: term.id, targetTermId: target, name: target ? "" : name, overwrite: Boolean(target) && overwrite }),
    onSuccess: (response) => {
      toast.success(t("copy.done", { count: response.copiedEntryCount }), { description: response.skippedEntryCount > 0 ? t("copy.skipped", { count: response.skippedEntryCount }) : undefined });
      refresh(queryClient, "timetable", "subjects");
      onOpenChange(false);
      void navigate({ to: "/school/timetable", search: { term: response.term?.id } });
    },
    onError: toastError,
  });
  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("copy.title")}
      description={t("copy.lead", { name: friend.displayName })}
      footer={
        <>
          <Button variant="ghost" onClick={() => onOpenChange(false)}>
            {t("common:actions.cancel")}
          </Button>
          <Button variant="lime" loading={copy.isPending} disabled={!target && !name.trim()} onClick={() => copy.mutate()}>
            {t("copy.action")}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-4">
        {ownTerms.length > 0 && (
          <SelectField
            label={t("copy.into")}
            value={target || "new"}
            onChange={(value) => setTarget(value === "new" ? "" : value)}
            options={[{ value: "new", label: t("copy.newTerm") }, ...ownTerms.map((own) => ({ value: own.id, label: own.name, hint: t("copy.existingHint") }))]}
          />
        )}
        {target ? (
          <div className="-mx-3">
            <SwitchRow label={t("copy.overwrite")} hint={t("copy.overwriteHint")} checked={overwrite} onChange={setOverwrite} />
          </div>
        ) : (
          <TextField label={t("terms.name")} value={name} maxLength={40} onChange={(event) => setName(event.target.value)} />
        )}
        <p className="rounded-2xl bg-surface-2 px-4 py-3 text-sm text-ink-soft">{t("copy.note")}</p>
      </div>
    </Dialog>
  );
}

/** A date inside a term nearest to today (for forms that need one). */
export function dateInTerm(term: Pick<school.Term, "startDate" | "endDate">): string {
  const today = ymd();
  if (today < term.startDate) return term.startDate;
  if (today > term.endDate) return term.endDate;
  return addDays(today, 0);
}
