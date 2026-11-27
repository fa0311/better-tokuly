import { studyTime } from "@tokuly/core/school";
import { router } from "expo-router";
import { Flame, Target } from "lucide-react-native";
import { useTranslation } from "react-i18next";
import { Pressable, Text, View } from "react-native";
import { Page, Spinner } from "#/components/page";
import { webScreenHref } from "#/features/web/handoff";
import { useTheme } from "#/lib/theme";
import { NS, useProgress } from "./data";
import { Block } from "./parts";
import { StudyingFriends } from "./studying-friends";
import { TimerCard } from "./timer-card";

/** `/school/study`: the timer, today against the goal, and the friends who are studying. */
export function TimerScreen() {
  const { t } = useTranslation(NS);
  return (
    <Page testID="school-study">
      <TimerCard />
      <ProgressCard />
      <Block title={t("study.friendsTitle")}>
        <StudyingFriends />
      </Block>
      <Pressable
        testID="school-study-web"
        onPress={() => router.push(webScreenHref("/school/study") as never)}
      >
        <Text className="py-2 text-center text-sm font-bold text-brand">
          {t("mobile:school.logOnWeb")}
        </Text>
      </Pressable>
    </Page>
  );
}

/** Today against the daily goal, the streak and the week. */
function ProgressCard() {
  const { t, i18n } = useTranslation(NS);
  const { colors } = useTheme();
  const { data, isPending } = useProgress();
  if (isPending) return <Spinner />;
  if (!data) return null;
  const goalSeconds = data.goalMinutes * 60;
  const fraction = goalSeconds > 0 ? Math.min(1, data.todaySeconds / goalSeconds) : 0;
  return (
    <View
      testID="school-progress"
      accessibilityLabel={t("study.progress")}
      className="gap-3 rounded-card bg-surface p-4"
    >
      <View className="flex-row items-end gap-4">
        <Figure value={studyTime(data.todaySeconds, i18n.language)} label={t("study.today")} />
        <Figure value={studyTime(data.weekSeconds, i18n.language)} label={t("study.thisWeek")} />
        <View className="items-end">
          <View className="flex-row items-center gap-1">
            <Flame size={18} color={data.streakDays > 0 ? colors.coral : colors["ink-faint"]} />
            <Text className="text-xl font-black text-ink">
              {t("study.streakDays", { count: data.streakDays })}
            </Text>
          </View>
          <Text className="text-xs font-bold text-ink-soft">
            {t("study.streak", { count: data.longestStreakDays })}
          </Text>
        </View>
      </View>
      {goalSeconds > 0 && (
        <View className="gap-1.5">
          <View
            accessibilityRole="progressbar"
            accessibilityLabel={t("study.goal")}
            accessibilityValue={{ min: 0, max: 100, now: Math.round(fraction * 100) }}
            className="h-4 overflow-hidden rounded-full border-2 border-ink bg-surface-3"
          >
            <View
              className={`h-full ${data.goalReached ? "bg-lime" : "bg-brand"}`}
              style={{ width: `${fraction * 100}%` }}
            />
          </View>
          <View className="flex-row items-center gap-1">
            <Target size={14} color={colors["ink-soft"]} />
            <Text className="text-xs font-bold text-ink-soft">
              {data.goalReached
                ? t("study.goalReached", { goal: studyTime(goalSeconds, i18n.language) })
                : t("study.goalLeft", {
                    goal: studyTime(goalSeconds, i18n.language),
                    left: studyTime(goalSeconds - data.todaySeconds, i18n.language),
                  })}
            </Text>
          </View>
        </View>
      )}
    </View>
  );
}

function Figure({ value, label }: { value: string; label: string }) {
  return (
    <View className="flex-1">
      <Text className="text-xl font-black text-ink">{value}</Text>
      <Text className="text-xs font-bold text-ink-soft">{label}</Text>
    </View>
  );
}
