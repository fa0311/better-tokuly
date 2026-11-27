import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { school } from "@tokuly/api";
import {
  isBreak,
  phaseKey,
  refreshSchool,
  schoolKeys,
  studyTime,
  type TimerView,
  timerText,
} from "@tokuly/core/school";
import { router } from "expo-router";
import { Pause, Play, SkipForward, Square } from "lucide-react-native";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Alert, AppState, Pressable, ScrollView, Text, View } from "react-native";
import Svg, { Circle } from "react-native-svg";
import { Button } from "#/components/button";
import { IconButton } from "#/components/icon-button";
import { Spinner } from "#/components/page";
import { Tabs } from "#/components/tabs";
import { TextField } from "#/components/text-field";
import { webScreenHref } from "#/features/web/handoff";
import { errorMessage } from "#/lib/errors";
import { INK_ON_BRIGHT, sticker, useTheme } from "#/lib/theme";
import { studiedSecondsAt, timerAt } from "./clock";
import { asAccount, NS, settingsClient, studyClient, timetableClient } from "./data";
import { SubjectPill } from "./parts";
import { loadSession, takeSession, timerAccount, useStudyTimer } from "./session";

type Mode = "stopwatch" | "pomodoro";
const MODES: Record<Mode, school.StudyMode> = {
  stopwatch: school.StudyMode.STOPWATCH,
  pomodoro: school.StudyMode.POMODORO,
};

const fail = (error: unknown) => Alert.alert(errorMessage(error, { namespace: NS }));

/** The phone's clock, again every `everyMs` while the app is on screen. */
function useTicker(everyMs: number): number {
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    const tick = () => setNow(Date.now());
    const timer = setInterval(tick, everyMs);
    const appState = AppState.addEventListener("change", (state) => {
      if (state === "active") tick();
    });
    return () => {
      clearInterval(timer);
      appState.remove();
    };
  }, [everyMs]);
  return now;
}

/**
 * The study timer. Idle, it offers a stopwatch or a pomodoro for a subject;
 * running, it shows the time the server counts with pause, skip and stop.
 */
export function TimerCard() {
  const { t } = useTranslation(NS);
  const { colors } = useTheme();
  const session = useStudyTimer((state) => state.session);
  const loaded = useStudyTimer((state) => state.loaded);
  const failed = useStudyTimer((state) => state.failed);
  const [retrying, setRetrying] = useState(false);

  const body = (() => {
    if (!loaded && failed)
      return (
        <View className="items-center gap-3 py-4">
          <Text className="text-4xl">🫠</Text>
          <Text className="font-black text-ink">{t("timer.loadFailed")}</Text>
          <Button
            testID="timer-retry"
            variant="secondary"
            loading={retrying}
            onPress={async () => {
              const account = timerAccount();
              if (!account) return;
              setRetrying(true);
              await loadSession(account);
              setRetrying(false);
            }}
          >
            {t("common:actions.retry")}
          </Button>
        </View>
      );
    if (!loaded) return <Spinner />;
    return session ? <Running session={session} /> : <Idle />;
  })();

  return (
    <View
      testID="school-timer"
      accessibilityLabel={t("timer.title")}
      className="gap-4 rounded-card bg-surface p-4"
      style={sticker(colors)}
    >
      {body}
    </View>
  );
}

function Idle() {
  const { t } = useTranslation(NS);
  const [mode, setMode] = useState<Mode>("stopwatch");
  const [subjectId, setSubjectId] = useState("");
  const [memo, setMemo] = useState("");
  const settings = useQuery({
    queryKey: schoolKeys.settings,
    queryFn: async () => (await settingsClient().getSchoolSettings({})).settings,
  });
  const start = useMutation({
    mutationFn: () =>
      asAccount(() => studyClient().startStudySession({ mode: MODES[mode], subjectId, memo })),
    onSuccess: (response) => {
      if (!response) return;
      takeSession(response.session, {
        heartbeatSeconds: response.heartbeatIntervalSeconds,
        idleSeconds: response.idleTimeoutSeconds,
      });
      setMemo("");
    },
    onError: fail,
  });
  const pomodoro = settings.data?.pomodoro;
  return (
    <>
      <Tabs
        testID="timer-mode"
        value={mode}
        onChange={setMode}
        items={[
          { value: "stopwatch", label: t("timer.stopwatch") },
          { value: "pomodoro", label: t("timer.pomodoro") },
        ]}
      />
      <Text className="px-1 text-sm text-ink-soft">
        {mode === "pomodoro"
          ? t("timer.pomodoroHint", {
              work: pomodoro?.workMinutes || 25,
              rest: pomodoro?.breakMinutes || 5,
              rounds: pomodoro?.rounds || 4,
            })
          : t("timer.stopwatchHint")}
      </Text>
      <SubjectChoice value={subjectId} onChange={setSubjectId} />
      <TextField
        testID="timer-memo"
        label={t("timer.memo")}
        value={memo}
        maxLength={200}
        placeholder={t("timer.memoPlaceholder")}
        onChangeText={setMemo}
      />
      <Button
        testID="timer-start"
        size="lg"
        block
        loading={start.isPending}
        onPress={() => start.mutate()}
      >
        {t("timer.start")}
      </Button>
    </>
  );
}

/** The subjects as a row of pills to pick from, with "none" first. */
function SubjectChoice({ value, onChange }: { value: string; onChange(id: string): void }) {
  const { t } = useTranslation(NS);
  const subjects = useQuery({
    queryKey: schoolKeys.subjects(false),
    queryFn: async () =>
      (await timetableClient().listSubjects({ includeArchived: false })).subjects,
  });
  const list = subjects.data ?? [];
  if (list.length === 0) return null;
  return (
    <View className="gap-1.5">
      <Text className="px-1 text-sm font-bold text-ink-soft">{t("mobile:school.subject")}</Text>
      <ScrollView horizontal showsHorizontalScrollIndicator={false}>
        <View className="flex-row gap-2 px-1 py-1">
          <Choice
            testID="timer-subject-none"
            chosen={value === ""}
            onPress={() => onChange("")}
            label={t("mobile:school.noSubject")}
          >
            <Text className="text-xs font-black text-ink">{t("mobile:school.noSubject")}</Text>
          </Choice>
          {list.map((subject) => (
            <Choice
              key={subject.id}
              testID={`timer-subject-${subject.id}`}
              chosen={value === subject.id}
              onPress={() => onChange(subject.id)}
              label={subject.name}
            >
              <SubjectPill subject={subject} />
            </Choice>
          ))}
        </View>
      </ScrollView>
    </View>
  );
}

function Choice({
  chosen,
  onPress,
  label,
  testID,
  children,
}: {
  chosen: boolean;
  onPress(): void;
  label: string;
  testID: string;
  children: React.ReactNode;
}) {
  return (
    <Pressable
      testID={testID}
      accessibilityRole="radio"
      accessibilityLabel={label}
      accessibilityState={{ selected: chosen }}
      onPress={onPress}
      className={`h-9 justify-center rounded-full px-1.5 ${chosen ? "bg-brand-soft border-2 border-brand" : "border-2 border-transparent"}`}
    >
      {children}
    </Pressable>
  );
}

function Running({ session }: { session: school.StudySession }) {
  const { t, i18n } = useTranslation(NS);
  const { colors } = useTheme();
  const queryClient = useQueryClient();
  const now = useTicker(250);
  const view = timerAt(session.timer, now);
  const studied = studiedSecondsAt(session, now);
  const paused = session.state === school.SessionState.PAUSED;
  const pomodoro = session.timer?.mode === school.StudyMode.POMODORO;
  const idleSeconds = useStudyTimer((state) => state.idleSeconds);

  const control = useMutation({
    mutationFn: (action: "pause" | "resume" | "skip") =>
      asAccount(async () => {
        const client = studyClient();
        const response =
          action === "pause"
            ? await client.pauseStudySession({})
            : action === "resume"
              ? await client.resumeStudySession({})
              : await client.skipStudyPhase({});
        return response.session;
      }),
    onSuccess: (next) => next && takeSession(next),
    onError: fail,
  });
  const stop = useMutation({
    mutationFn: () => asAccount(() => studyClient().stopStudySession({})),
    onSuccess: (response) => {
      if (!response) return;
      takeSession(undefined);
      refreshSchool(queryClient, "study", "studying");
      const seconds = response.session?.studiedSeconds ?? 0;
      const progress = response.progress;
      const reachedNow =
        progress?.goalReached && progress.todaySeconds - seconds < progress.goalMinutes * 60;
      Alert.alert(
        reachedNow
          ? t("timer.goalReached")
          : t("timer.stopped", { time: studyTime(seconds, i18n.language) }),
      );
    },
    onError: fail,
  });
  const confirmStop = () =>
    Alert.alert(t("timer.stopTitle"), t("timer.stopBody"), [
      { text: t("timer.keepGoing"), style: "cancel" },
      { text: t("timer.finish"), onPress: () => stop.mutate() },
    ]);

  return (
    <>
      <TimerFace view={view} studiedSeconds={studied} />
      <View className="flex-row flex-wrap items-center justify-center gap-2">
        <SubjectPill subject={session.subject} />
        {session.memo ? (
          <Text numberOfLines={1} className="max-w-full text-sm text-ink-soft">
            {session.memo}
          </Text>
        ) : null}
        {session.cheerCount > 0 && (
          <Text
            testID="timer-cheers"
            className="overflow-hidden rounded-full bg-sun px-2.5 py-0.5 text-sm font-black"
            style={{ color: INK_ON_BRIGHT }}
          >
            📣 {session.cheerCount}
          </Text>
        )}
      </View>
      {session.roomId ? (
        <View className="gap-2">
          <Text className="text-center text-sm text-ink-soft">{t("timer.inRoom")}</Text>
          <Button
            testID="timer-room"
            block
            onPress={() => router.push(webScreenHref(`/school/rooms/${session.roomId}`) as never)}
          >
            {t("timer.backToRoom")}
          </Button>
        </View>
      ) : (
        <View className="flex-row items-center justify-center gap-3">
          {paused ? (
            <Button
              testID="timer-resume"
              variant="lime"
              icon={<Play size={16} color={INK_ON_BRIGHT} />}
              loading={control.isPending}
              onPress={() => control.mutate("resume")}
            >
              {t("timer.resume")}
            </Button>
          ) : (
            <Button
              testID="timer-pause"
              variant="secondary"
              icon={<Pause size={16} color={colors.ink} />}
              loading={control.isPending}
              onPress={() => control.mutate("pause")}
            >
              {t("timer.pause")}
            </Button>
          )}
          {pomodoro && !paused && (
            <IconButton
              testID="timer-skip"
              label={t("timer.skip")}
              icon={<SkipForward size={20} color={colors.ink} />}
              disabled={control.isPending}
              onPress={() => control.mutate("skip")}
            />
          )}
          <Button
            testID="timer-stop"
            variant="danger"
            icon={<Square size={16} color="#ffffff" />}
            loading={stop.isPending}
            disabled={control.isPending}
            onPress={confirmStop}
          >
            {t("timer.stop")}
          </Button>
        </View>
      )}
      <Text className="text-center text-xs text-ink-faint">
        {t("mobile:school.timerNote", { minutes: Math.round(idleSeconds / 60) })}
      </Text>
    </>
  );
}

const RADIUS = 54;
const CIRCUMFERENCE = 2 * Math.PI * RADIUS;

/**
 * The face of a timer: the digits (counting up for a stopwatch, down for a
 * pomodoro phase) inside a ring that empties with the phase.
 */
function TimerFace({ view, studiedSeconds }: { view: TimerView; studiedSeconds: number }) {
  const { t, i18n } = useTranslation(NS);
  const { colors } = useTheme();
  const pomodoro = view.mode === school.StudyMode.POMODORO;
  const running = view.state === school.TimerState.RUNNING;
  const tint = !running ? colors["ink-faint"] : isBreak(view.phase) ? colors.sky : colors.brand;
  const fraction =
    pomodoro && view.phaseMs > 0 ? Math.min(1, Math.max(0, view.remainingMs / view.phaseMs)) : 1;
  const digits = pomodoro
    ? timerText(Math.ceil(view.remainingMs / 1000))
    : timerText(studiedSeconds);
  return (
    <View className="aspect-square w-full max-w-64 items-center justify-center self-center">
      <Svg
        viewBox="0 0 120 120"
        style={{
          position: "absolute",
          width: "100%",
          height: "100%",
          transform: [{ rotate: "-90deg" }],
        }}
      >
        <Circle
          cx="60"
          cy="60"
          r={RADIUS}
          fill="none"
          strokeWidth={9}
          stroke={colors["surface-3"]}
        />
        <Circle
          cx="60"
          cy="60"
          r={RADIUS}
          fill="none"
          strokeWidth={9}
          strokeLinecap="round"
          stroke={tint}
          strokeDasharray={CIRCUMFERENCE}
          strokeDashoffset={CIRCUMFERENCE * (1 - fraction)}
        />
      </Svg>
      <Text className="text-sm font-black" style={{ color: tint }}>
        {t(`timer.phase.${phaseKey(view)}`)}
        {pomodoro && view.state !== school.TimerState.IDLE
          ? `  ${t("timer.round", { round: ((view.round - 1) % view.rounds) + 1, rounds: view.rounds })}`
          : ""}
      </Text>
      <Text
        testID="timer-clock"
        accessibilityLabel={t("timer.clock")}
        className="font-black text-ink"
        style={{ fontSize: 46, fontVariant: ["tabular-nums"] }}
      >
        {digits}
      </Text>
      {pomodoro && (
        <Text className="mt-1 text-xs font-bold text-ink-soft">
          {t("timer.studied", { time: studyTime(studiedSeconds, i18n.language) })}
        </Text>
      )}
    </View>
  );
}

/** The way to the timer on the school screen: what it shows now, or an invitation to start. */
export function TimerEntry() {
  const { t } = useTranslation(NS);
  const { colors } = useTheme();
  const session = useStudyTimer((state) => state.session);
  return (
    <Pressable
      testID="school-timer-entry"
      accessibilityRole="button"
      onPress={() => router.push("/school/study")}
      className="flex-row items-center gap-3 rounded-card bg-brand px-4 py-3"
      style={sticker(colors)}
    >
      <Text className="text-2xl">{session ? "🔥" : "⏱️"}</Text>
      <View className="flex-1">
        <Text className="font-black text-on-brand">{t("timer.title")}</Text>
        {session ? (
          <EntryClock session={session} />
        ) : (
          <Text className="text-sm font-bold text-on-brand">{t("mobile:school.timerStart")}</Text>
        )}
      </View>
      <Play size={20} color={colors["on-brand"]} />
    </Pressable>
  );
}

function EntryClock({ session }: { session: school.StudySession }) {
  const { t } = useTranslation(NS);
  const now = useTicker(1000);
  const view = timerAt(session.timer, now);
  return (
    <Text className="text-sm font-bold text-on-brand" style={{ fontVariant: ["tabular-nums"] }}>
      {t(`timer.phase.${phaseKey(view)}`)} · {timerText(studiedSecondsAt(session, now))}
    </Text>
  );
}
