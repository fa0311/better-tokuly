import { router } from "expo-router";
import { useTranslation } from "react-i18next";
import { Pressable, Text, View } from "react-native";
import { Button } from "#/components/button";
import type { ProfileSection, ProfileSectionProps } from "#/seams/profile";
import { useAnswers, useBox } from "./data";
import { QaCard } from "./parts";

/** How many answers a profile shows before "more". */
const SHOWN = 3;

/** "質問する" on someone whose box is open, "質問箱" on one's own profile. */
function ProfileAskButton({ user, isSelf }: ProfileSectionProps) {
  const { t } = useTranslation("ask");
  const { data: box } = useBox(isSelf ? "" : user.id);
  if (isSelf)
    return (
      <View className="items-center">
        <Button
          testID="profile-ask"
          variant="lime"
          onPress={() => router.push("/ask")}
          icon={<Text>💌</Text>}
        >
          {t("profile.myBox")}
        </Button>
      </View>
    );
  if (!box?.open) return null;
  return (
    <View className="items-center">
      <Button
        testID="profile-ask"
        variant="lime"
        onPress={() => router.push(`/ask/u/${user.handle}`)}
        icon={<Text>💌</Text>}
      >
        {t("profile.ask")}
      </Button>
    </View>
  );
}

/** The newest answers a person published. Nothing while there are none. */
function ProfileAnswers({ user }: ProfileSectionProps) {
  const { t } = useTranslation("ask");
  const answers = useAnswers(user.id);
  const questions = answers.data?.pages[0]?.questions ?? [];
  if (questions.length === 0) return null;
  const more = questions.length > SHOWN || Boolean(answers.hasNextPage);
  return (
    <View testID="profile-answers" className="gap-3">
      <View className="flex-row items-center justify-between px-1">
        <Text className="text-xs font-black tracking-wider text-ink-soft uppercase">
          💌 {t("profile.title")}
        </Text>
        {more && (
          <Pressable
            testID="profile-answers-all"
            accessibilityRole="link"
            onPress={() => router.push(`/ask/u/${user.handle}`)}
          >
            <Text className="text-sm font-black text-brand">{t("profile.more")}</Text>
          </Pressable>
        )}
      </View>
      {questions.slice(0, SHOWN).map((question) => (
        <QaCard key={question.id} question={question} />
      ))}
    </View>
  );
}

export default [
  { id: "ask-button", slot: "action", order: 30, Component: ProfileAskButton },
  { id: "ask-answers", slot: "content", order: 40, Component: ProfileAnswers },
] satisfies ProfileSection[];
