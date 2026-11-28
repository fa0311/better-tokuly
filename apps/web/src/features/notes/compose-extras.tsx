import { notes } from "@tokuly/api";
import { POLL_DURATIONS, POLL_MAX_CHOICES, type PollDraft } from "@tokuly/core/notes";
import { Plus, X } from "lucide-react";
import { cn } from "#/lib/cn";
import { useTranslation } from "#/lib/use-translation";

const FIELD =
  "h-9 min-w-0 rounded-xl border-2 border-line bg-surface px-3 text-sm font-bold outline-none focus:border-ink";

/** The poll editor under the text: 2–10 choices, single or multiple, how long, who sees the results when. */
export function PollEditor({
  poll,
  onChange,
  onRemove,
}: {
  poll: PollDraft;
  onChange(poll: PollDraft): void;
  onRemove(): void;
}) {
  const { t } = useTranslation("notes");
  const setChoice = (index: number, value: string) =>
    onChange({ ...poll, choices: poll.choices.map((choice, i) => (i === index ? value : choice)) });
  return (
    <fieldset
      aria-label={t("compose.poll.title")}
      className="mt-3 flex flex-col gap-2 rounded-2xl border-2 border-line p-3"
    >
      {poll.choices.map((choice, index) => (
        <div key={index} className="flex items-center gap-2">
          <input
            value={choice}
            maxLength={50}
            aria-label={t("compose.poll.choice", { index: index + 1 })}
            placeholder={t("compose.poll.choice", { index: index + 1 })}
            onChange={(event) => setChoice(index, event.target.value)}
            className={cn(FIELD, "flex-1")}
          />
          {poll.choices.length > 2 && (
            <button
              type="button"
              aria-label={t("compose.poll.removeChoice")}
              onClick={() =>
                onChange({ ...poll, choices: poll.choices.filter((_, i) => i !== index) })
              }
              className="grid size-8 place-items-center rounded-full hover:bg-surface-2"
            >
              <X className="size-4" />
            </button>
          )}
        </div>
      ))}
      {poll.choices.length < POLL_MAX_CHOICES && (
        <button
          type="button"
          onClick={() => onChange({ ...poll, choices: [...poll.choices, ""] })}
          className="inline-flex w-fit items-center gap-1 text-sm font-black text-brand hover:underline"
        >
          <Plus className="size-4" />
          {t("compose.poll.addChoice")}
        </button>
      )}
      <div className="flex flex-wrap items-center gap-2 text-sm font-bold">
        <label className="flex items-center gap-1.5">
          <input
            type="checkbox"
            checked={poll.multiple}
            onChange={(event) => onChange({ ...poll, multiple: event.target.checked })}
          />
          {t("compose.poll.multiple")}
        </label>
        <select
          aria-label={t("compose.poll.duration")}
          value={poll.durationSeconds}
          onChange={(event) => onChange({ ...poll, durationSeconds: Number(event.target.value) })}
          className={FIELD}
        >
          {POLL_DURATIONS.map((seconds) => (
            <option key={seconds} value={seconds}>
              {t(`compose.poll.durations.${seconds}`)}
            </option>
          ))}
        </select>
        <select
          aria-label={t("compose.poll.results")}
          value={poll.results}
          onChange={(event) => onChange({ ...poll, results: Number(event.target.value) })}
          className={FIELD}
        >
          <option value={notes.PollResultsVisibility.ALWAYS}>
            {t("compose.poll.resultsAlways")}
          </option>
          <option value={notes.PollResultsVisibility.AFTER_VOTING}>
            {t("compose.poll.resultsAfterVoting")}
          </option>
          <option value={notes.PollResultsVisibility.AFTER_END}>
            {t("compose.poll.resultsAfterEnd")}
          </option>
        </select>
        <span className="flex-1" />
        <button
          type="button"
          onClick={onRemove}
          className="text-sm font-black text-danger hover:underline"
        >
          {t("compose.poll.remove")}
        </button>
      </div>
    </fieldset>
  );
}

/** The value of a `datetime-local` input for a time (local clock). */
export function localDateTime(ms: number): string {
  const date = new Date(ms - new Date(ms).getTimezoneOffset() * 60_000);
  return date.toISOString().slice(0, 16);
}

/** Choosing when a scheduled note is posted: 5 minutes to 90 days ahead. */
export function ScheduleField({
  value,
  onChange,
}: {
  value?: number;
  onChange(value?: number): void;
}) {
  const { t } = useTranslation("notes");
  const now = Date.now();
  return (
    <div className="mt-3 flex flex-wrap items-center gap-2 rounded-2xl border-2 border-dashed border-line px-3 py-2 text-sm font-bold">
      <label className="flex items-center gap-2">
        {t("compose.schedule.label")}
        <input
          type="datetime-local"
          aria-label={t("compose.schedule.label")}
          min={localDateTime(now + 5 * 60_000)}
          max={localDateTime(now + 90 * 86_400_000)}
          value={value ? localDateTime(value) : ""}
          onChange={(event) =>
            onChange(event.target.value ? new Date(event.target.value).getTime() : undefined)
          }
          className={FIELD}
        />
      </label>
      <button
        type="button"
        onClick={() => onChange(undefined)}
        className="text-sm font-black text-danger hover:underline"
      >
        {t("compose.schedule.cancel")}
      </button>
    </div>
  );
}
