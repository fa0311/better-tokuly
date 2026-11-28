import { type notify, toMillis } from "@tokuly/api";
import { CirclePause, Play } from "lucide-react";
import { Button } from "#/components/ui/button";
import { Card, Chip } from "#/components/ui/card";
import { cn } from "#/lib/cn";
import { useTranslation } from "#/lib/use-translation";
import { usePreferenceActions } from "../preferences";
import { useNow } from "../when";

/** How long a pause can be asked for, in seconds; 0 = until resumed. */
const DURATIONS = [
  { seconds: 30 * 60, key: "pause.durations.30m" },
  { seconds: 60 * 60, key: "pause.durations.1h" },
  { seconds: 8 * 60 * 60, key: "pause.durations.8h" },
  { seconds: 24 * 60 * 60, key: "pause.durations.24h" },
  { seconds: 0, key: "pause.durations.manual" },
] as const;

/** Pause every push for a while (the inbox keeps filling), or resume. */
export function PauseCard({ preferences }: { preferences: notify.Preferences }) {
  const { t, i18n } = useTranslation("notifications");
  const actions = usePreferenceActions();
  const now = useNow(15_000);
  const until = toMillis(preferences.pushPause?.until);
  // A timed pause ends by itself; do not wait for the next fetch to say so.
  const paused = Boolean(preferences.pushPause?.paused) && (until === 0 || until > now);
  const busy = actions.pause.isPending || actions.resume.isPending;

  return (
    <Card>
      <div className="flex items-start gap-3">
        <span
          className={cn(
            "sticker-sm grid size-12 shrink-0 place-items-center rounded-2xl",
            paused ? "bg-sun text-[#17122b]" : "bg-surface-3 text-ink",
          )}
        >
          <CirclePause className="size-6" />
        </span>
        <div className="min-w-0 flex-1">
          <h2 className="flex flex-wrap items-center gap-2 text-lg font-black leading-snug">
            {t("pause.title")}
            {paused && <Chip className="bg-sun text-[#17122b]">{t("pause.active")}</Chip>}
          </h2>
          <p className="mt-1 text-sm leading-relaxed text-ink-soft" aria-live="polite">
            {paused
              ? until
                ? t("pause.until", { when: formatUntil(until, now, i18n.language) })
                : t("pause.untilResumed")
              : t("pause.lead")}
          </p>
        </div>
      </div>
      {paused ? (
        <Button
          variant="lime"
          className="mt-4"
          icon={<Play className="size-4" />}
          loading={actions.resume.isPending}
          onClick={() => actions.resume.mutate()}
        >
          {t("pause.resume")}
        </Button>
      ) : (
        <fieldset className="mt-4 flex flex-wrap gap-2" disabled={busy}>
          <legend className="sr-only">{t("pause.choose")}</legend>
          {DURATIONS.map((duration) => (
            <Button
              key={duration.seconds}
              variant="secondary"
              size="sm"
              onClick={() => actions.pause.mutate(duration.seconds)}
            >
              {t(duration.key)}
            </Button>
          ))}
        </fieldset>
      )}
    </Card>
  );
}

/** "15:30" today, "11月23日 15:30" otherwise. */
function formatUntil(untilMs: number, nowMs: number, language: string): string {
  const until = new Date(untilMs);
  const sameDay = until.toDateString() === new Date(nowMs).toDateString();
  return sameDay
    ? until.toLocaleTimeString(language, { hour: "2-digit", minute: "2-digit" })
    : until.toLocaleString(language, {
        month: "short",
        day: "numeric",
        hour: "2-digit",
        minute: "2-digit",
      });
}
