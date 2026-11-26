import { useMutation } from "@tanstack/react-query";
import { errorReason, school } from "@tokuly/api";
import { studyTime } from "@tokuly/core/school";
import { router } from "expo-router";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Alert, Pressable, Text, View } from "react-native";
import { Avatar } from "#/components/avatar";
import { Button } from "#/components/button";
import { Spinner } from "#/components/page";
import { errorMessage } from "#/lib/errors";
import { INK_ON_BRIGHT } from "#/lib/theme";
import { asAccount, NS, studyClient, useAccountNow, useStudyingFriends } from "./data";
import { SubjectPill } from "./parts";

const isBreak = (phase: school.TimerPhase) =>
  phase === school.TimerPhase.BREAK || phase === school.TimerPhase.LONG_BREAK;

/**
 * Friends who are studying right now, as the server lets the caller see
 * them (a friend who hides their studying is not in the list), each with
 * 「がんばれ!」.
 */
export function StudyingFriends() {
  const { t, i18n } = useTranslation(NS);
  const { data, isPending, dataUpdatedAt } = useStudyingFriends();
  const { ms: now } = useAccountNow(30_000);
  if (isPending) return <Spinner />;
  const friends = data ?? [];
  if (friends.length === 0)
    return (
      <Text
        testID="school-studying-empty"
        className="rounded-card bg-surface-2 px-4 py-5 text-center text-sm font-bold text-ink-soft"
      >
        {t("study.nobodyStudying")}
      </Text>
    );
  return (
    <View testID="school-studying" className="gap-1.5">
      {friends.map((friend) => {
        const running = friend.state === school.SessionState.RUNNING && !isBreak(friend.phase);
        // Counted on this phone's clock from when the list arrived: the phone's offset from the server does not matter.
        const seconds =
          friend.studiedSeconds +
          (running ? Math.max(0, Math.floor((now - dataUpdatedAt) / 1000)) : 0);
        const name = friend.user?.displayName ?? "";
        return (
          <View
            key={friend.sessionId}
            className="flex-row items-center gap-3 rounded-2xl bg-surface-2 px-3 py-2.5"
          >
            <Pressable
              accessibilityRole="link"
              accessibilityLabel={name}
              onPress={() => friend.user && router.push(`/u/${friend.user.handle}`)}
            >
              <Avatar user={friend.user} />
            </Pressable>
            <View className="flex-1 gap-0.5">
              <Text numberOfLines={1} className="font-bold text-ink">
                {name}
              </Text>
              <View className="flex-row flex-wrap items-center gap-1.5">
                <Text
                  className={`overflow-hidden rounded-full px-2 py-0.5 text-[11px] font-black ${running ? "bg-brand text-on-brand" : "bg-sky"}`}
                  style={running ? undefined : { color: INK_ON_BRIGHT }}
                >
                  {running ? t("study.studying") : t("study.resting")}
                </Text>
                <SubjectPill subject={friend.subject} />
                <Text className="text-xs font-bold text-ink-soft">
                  {studyTime(seconds, i18n.language)}
                </Text>
              </View>
            </View>
            {friend.user && <CheerButton userId={friend.user.id} name={name} />}
          </View>
        );
      })}
    </View>
  );
}

/** Sends 「がんばれ!」 to a friend who is studying; one per friend every few minutes. */
function CheerButton({ userId, name }: { userId: string; name: string }) {
  const { t } = useTranslation(NS);
  const [sent, setSent] = useState(false);
  const cheer = useMutation({
    mutationFn: () => asAccount(() => studyClient().cheerStudy({ userId })),
    onSuccess: (answer) => {
      if (answer) setSent(true);
    },
    onError: (error) => {
      // The last cheer still counts.
      if (errorReason(error) === "RATE_LIMITED") {
        setSent(true);
        Alert.alert(t("study.cheerTooSoon"));
      } else Alert.alert(errorMessage(error, { namespace: NS }));
    },
  });
  return (
    <Button
      testID={sent ? `cheered-${userId}` : `cheer-${userId}`}
      size="sm"
      variant={sent ? "secondary" : "lime"}
      loading={cheer.isPending}
      disabled={sent}
      accessibilityLabel={t("study.cheerFor", { name })}
      onPress={() => cheer.mutate()}
    >
      {sent ? t("study.cheered_") : t("study.cheer")}
    </Button>
  );
}
