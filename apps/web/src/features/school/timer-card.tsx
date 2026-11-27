import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { school, toDate } from "@tokuly/api";
import { phaseKey, refreshSchool, schoolKeys, studyTime, timerText } from "@tokuly/core/school";
import { DoorOpen, Pause, Play, RotateCw, SkipForward, Square } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { Dialog } from "#/components/ui/dialog";
import { TextField } from "#/components/ui/field";
import { Skeleton } from "#/components/ui/page";
import { useAccountTimeZone } from "#/lib/account-time";
import { cn } from "#/lib/cn";
import { studyClient, toastError, useSchoolSettings } from "./api";

import { Segmented, SubjectPill, useNow } from "./parts";
import { loadActiveSession, setActiveSession, useActiveSession } from "./session-store";
import { SubjectSelect } from "./subject-select";
import { isBreak, studiedSecondsAt, type TimerView, timerAt } from "./timer";

/**
 * The face of a timer: the digits (counting up for a stopwatch, down for a
 * pomodoro phase) inside a ring that empties with the phase.
 */
export function TimerFace({
  view,
  studiedSeconds,
  className,
}: {
  view: TimerView /** Shown under a pomodoro's countdown. */;
  studiedSeconds?: number;
  className?: string;
}) {
  const { t, i18n } = useTranslation("school");
  const pomodoro = view.mode === school.StudyMode.POMODORO;
  const resting = isBreak(view.phase);
  const digits = pomodoro
    ? timerText(Math.ceil(view.remainingMs / 1000))
    : timerText(studiedSeconds ?? view.elapsedMs / 1000);
  const fraction =
    pomodoro && view.phaseMs > 0 ? Math.min(1, Math.max(0, view.remainingMs / view.phaseMs)) : 1;
  const radius = 54;
  const circumference = 2 * Math.PI * radius;
  const running = view.state === school.TimerState.RUNNING;
  return (
    <div
      className={cn(
        "relative mx-auto grid aspect-square w-full max-w-64 place-items-center",
        className,
      )}
    >
      <svg viewBox="0 0 120 120" className="absolute inset-0 size-full -rotate-90" aria-hidden>
        <circle
          cx="60"
          cy="60"
          r={radius}
          fill="none"
          strokeWidth="9"
          className="stroke-surface-3"
        />
        <circle
          cx="60"
          cy="60"
          r={radius}
          fill="none"
          strokeWidth="9"
          strokeLinecap="round"
          strokeDasharray={circumference}
          strokeDashoffset={circumference * (1 - fraction)}
          className={cn(
            "transition-[stroke-dashoffset] duration-500",
            !running ? "stroke-ink-faint" : resting ? "stroke-sky" : "stroke-brand",
          )}
        />
      </svg>
      <div className="relative text-center">
        <p
          className={cn(
            "text-sm font-black",
            !running ? "text-ink-soft" : resting ? "text-sky" : "text-brand",
          )}
        >
          {t(`timer.phase.${phaseKey(view)}`)}
          {pomodoro && view.state !== school.TimerState.IDLE && (
            <span className="ml-1.5 text-ink-faint">
              {t("timer.round", {
                round: ((view.round - 1) % view.rounds) + 1,
                rounds: view.rounds,
              })}
            </span>
          )}
        </p>
        <p
          role="timer"
          aria-label={t("timer.clock")}
          className="font-black tabular-nums leading-none tracking-tight text-[2.75rem]"
        >
          {digits}
        </p>
        {pomodoro && studiedSeconds !== undefined && (
          <p className="mt-1 text-xs font-bold text-ink-soft">
            {t("timer.studied", { time: studyTime(studiedSeconds, i18n.language) })}
          </p>
        )}
      </div>
    </div>
  );
}

/**
 * The study timer. Idle, it offers a stopwatch or a pomodoro for a subject;
 * running, it shows the time the server counts (it keeps running in a
 * background tab and comes back after a reload) with pause, skip and stop.
 */
export function TimerCard() {
  const { t } = useTranslation("school");
  const session = useActiveSession((state) => state.session);
  const loaded = useActiveSession((state) => state.loaded);
  const failed = useActiveSession((state) => state.failed);
  const settings = useSchoolSettings();
  const [mode, setMode] = useState<school.StudyMode>(school.StudyMode.STOPWATCH);
  const [subjectId, setSubjectId] = useState("");
  const [memo, setMemo] = useState("");
  const [stopping, setStopping] = useState(false);
  const [cheersOpen, setCheersOpen] = useState(false);

  const start = useMutation({
    mutationFn: () => studyClient().startStudySession({ mode, subjectId, memo }),
    onSuccess: (response) => {
      setActiveSession(response.session, response.heartbeatIntervalSeconds);
      setMemo("");
    },
    onError: toastError,
  });
  const retry = useMutation({
    mutationFn: () => loadActiveSession(() => studyClient().getCurrentStudySession({})),
  });
  const control = useMutation({
    mutationFn: async (action: "pause" | "resume" | "skip") => {
      const client = studyClient();
      const response =
        action === "pause"
          ? await client.pauseStudySession({})
          : action === "resume"
            ? await client.resumeStudySession({})
            : await client.skipStudyPhase({});
      return response.session;
    },
    onSuccess: (next) => setActiveSession(next),
    onError: toastError,
  });

  if (!loaded && failed) {
    return (
      <section
        aria-label={t("timer.title")}
        className="sticker flex flex-col items-center gap-3 rounded-card bg-surface p-6 text-center"
      >
        <span className="text-4xl">🫠</span>
        <p className="font-black">{t("timer.loadFailed")}</p>
        <Button
          variant="secondary"
          icon={<RotateCw className="size-4" />}
          loading={retry.isPending}
          onClick={() => retry.mutate()}
        >
          {t("common:actions.retry")}
        </Button>
      </section>
    );
  }
  if (!loaded) return <Skeleton className="h-80" />;

  if (!session) {
    const pomodoro = settings.data?.pomodoro;
    return (
      <section
        aria-label={t("timer.title")}
        className="sticker flex flex-col gap-4 rounded-card bg-surface p-5"
      >
        <Segmented
          label={t("timer.mode")}
          className="w-full"
          value={mode}
          onChange={(next: school.StudyMode) => setMode(next)}
          options={[
            { value: school.StudyMode.STOPWATCH, label: t("timer.stopwatch") },
            { value: school.StudyMode.POMODORO, label: t("timer.pomodoro") },
          ]}
        />
        <p className="px-1 text-sm text-ink-soft">
          {mode === school.StudyMode.POMODORO
            ? t("timer.pomodoroHint", {
                work: pomodoro?.workMinutes || 25,
                rest: pomodoro?.breakMinutes || 5,
                rounds: pomodoro?.rounds || 4,
              })
            : t("timer.stopwatchHint")}
          {mode === school.StudyMode.POMODORO && (
            <Link to="/school/settings" className="ml-1 font-bold text-brand">
              {t("timer.changeLengths")}
            </Link>
          )}
        </p>
        <SubjectSelect value={subjectId} onChange={setSubjectId} />
        <TextField
          label={t("timer.memo")}
          value={memo}
          maxLength={200}
          placeholder={t("timer.memoPlaceholder")}
          onChange={(event) => setMemo(event.target.value)}
        />
        <Button
          size="lg"
          block
          icon={<Play className="size-5" />}
          loading={start.isPending}
          onClick={() => start.mutate()}
        >
          {t("timer.start")}
        </Button>
      </section>
    );
  }

  return (
    <section
      aria-label={t("timer.title")}
      className="sticker flex flex-col gap-4 rounded-card bg-surface p-5"
    >
      <RunningFace session={session} />
      <div className="flex flex-wrap items-center justify-center gap-2">
        <SubjectPill subject={session.subject} className="h-7 px-2.5 text-sm" />
        {session.memo && (
          <span className="max-w-full truncate text-sm text-ink-soft">{session.memo}</span>
        )}
        {session.cheerCount > 0 && (
          <button
            type="button"
            onClick={() => setCheersOpen(true)}
            className="inline-flex h-7 items-center gap-1 rounded-full bg-sun px-2.5 text-sm font-black text-[#17122b]"
          >
            📣 {session.cheerCount}
          </button>
        )}
      </div>
      {session.roomId ? (
        <div className="flex flex-col gap-2">
          <p className="text-center text-sm text-ink-soft">{t("timer.inRoom")}</p>
          <Link
            to="/school/rooms/$roomId"
            params={{ roomId: session.roomId }}
            className="pressable sticker inline-flex h-11 items-center justify-center gap-2 rounded-full bg-brand px-5 font-bold text-on-brand"
          >
            <DoorOpen className="size-4" />
            {t("timer.backToRoom")}
          </Link>
        </div>
      ) : (
        <div className="flex items-center justify-center gap-3">
          {session.state === school.SessionState.PAUSED ? (
            <Button
              variant="lime"
              icon={<Play className="size-4" />}
              loading={control.isPending}
              onClick={() => control.mutate("resume")}
            >
              {t("timer.resume")}
            </Button>
          ) : (
            <Button
              variant="secondary"
              icon={<Pause className="size-4" />}
              loading={control.isPending}
              onClick={() => control.mutate("pause")}
            >
              {t("timer.pause")}
            </Button>
          )}
          {session.timer?.mode === school.StudyMode.POMODORO &&
            session.state === school.SessionState.RUNNING && (
              <Button
                variant="ghost"
                size="icon"
                aria-label={t("timer.skip")}
                disabled={control.isPending}
                onClick={() => control.mutate("skip")}
              >
                <SkipForward className="size-5" />
              </Button>
            )}
          <Button
            variant="danger"
            icon={<Square className="size-4" />}
            disabled={control.isPending}
            onClick={() => setStopping(true)}
          >
            {t("timer.stop")}
          </Button>
        </div>
      )}
      {stopping && <StopDialog session={session} onClose={() => setStopping(false)} />}
      {cheersOpen && <CheersDialog sessionId={session.id} onClose={() => setCheersOpen(false)} />}
      <p className="text-center text-xs text-ink-faint">{t("timer.keepsRunning")}</p>
    </section>
  );
}

function RunningFace({ session }: { session: school.StudySession }) {
  const now = useNow(250);
  return (
    <TimerFace view={timerAt(session.timer, now)} studiedSeconds={studiedSecondsAt(session, now)} />
  );
}

/** Ends the session, with a last chance to note what was done. */
function StopDialog({ session, onClose }: { session: school.StudySession; onClose(): void }) {
  const { t, i18n } = useTranslation("school");
  const queryClient = useQueryClient();
  const [memo, setMemo] = useState(session.memo);
  const stop = useMutation({
    mutationFn: () => studyClient().stopStudySession({ memo }),
    onSuccess: (response) => {
      setActiveSession(undefined);
      refreshSchool(queryClient, "study", "studying");
      const seconds = response.session?.studiedSeconds ?? 0;
      if (
        response.progress?.goalReached &&
        response.progress.todaySeconds - seconds < response.progress.goalMinutes * 60
      )
        toast.success(t("timer.goalReached"), { icon: "🎉" });
      else toast.success(t("timer.stopped", { time: studyTime(seconds, i18n.language) }));
      onClose();
    },
    onError: toastError,
  });
  return (
    <Dialog
      open
      onOpenChange={(open) => !open && onClose()}
      title={t("timer.stopTitle")}
      description={t("timer.stopBody")}
      footer={
        <>
          <Button variant="ghost" onClick={onClose}>
            {t("timer.keepGoing")}
          </Button>
          <Button variant="lime" loading={stop.isPending} onClick={() => stop.mutate()}>
            {t("timer.finish")}
          </Button>
        </>
      }
    >
      <TextField
        label={t("timer.memo")}
        value={memo}
        maxLength={200}
        placeholder={t("timer.memoPlaceholder")}
        onChange={(event) => setMemo(event.target.value)}
      />
    </Dialog>
  );
}

/** Who cheered during a session. */
export function CheersDialog({ sessionId, onClose }: { sessionId: string; onClose(): void }) {
  const { t, i18n } = useTranslation("school");
  const zone = useAccountTimeZone();
  const cheers = useQuery({
    queryKey: schoolKeys.cheers(sessionId),
    queryFn: async () =>
      (await studyClient().listStudyCheers({ sessionId, page: { limit: 100 } })).cheers,
  });
  return (
    <Dialog open onOpenChange={(open) => !open && onClose()} title={t("study.cheersTitle")}>
      {cheers.isPending ? (
        <Skeleton className="h-14" />
      ) : (cheers.data ?? []).length === 0 ? (
        <p className="text-sm text-ink-soft">{t("study.noCheers")}</p>
      ) : (
        <ul className="flex flex-col gap-2">
          {(cheers.data ?? []).map((cheer) => (
            <li key={cheer.id} className="flex items-center gap-3">
              <Avatar user={cheer.from} size="sm" />
              <span className="min-w-0 flex-1 truncate font-bold">
                {t("study.cheerFrom", { name: cheer.from?.displayName ?? "" })}
              </span>
              <span className="text-xs text-ink-faint">
                {/* The account's clock, like every other time of the school day. */}
                {toDate(cheer.createdAt)?.toLocaleTimeString(i18n.language, {
                  hour: "numeric",
                  minute: "2-digit",
                  timeZone: zone,
                })}
              </span>
            </li>
          ))}
        </ul>
      )}
    </Dialog>
  );
}
