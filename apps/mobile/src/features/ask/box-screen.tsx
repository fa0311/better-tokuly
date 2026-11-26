import { router } from "expo-router";
import { Inbox } from "lucide-react-native";
import { useTranslation } from "react-i18next";
import { Pressable, Text, View } from "react-native";
import { Avatar } from "#/components/avatar";
import { Button } from "#/components/button";
import { EmptyState, Page, Spinner } from "#/components/page";
import { useMe } from "#/features/session/store";
import { INK_ON_BRIGHT, sticker, useTheme } from "#/lib/theme";
import { AskForm } from "./ask-form";
import { useAnswers, useBox } from "./data";
import { MoreButton, QaCard } from "./parts";
import { ShareBoxButton } from "./share";

/** `/ask/u/<handle>`: someone's question box. Ask them something, read what they answered. */
export function BoxScreen({ handle }: { handle: string }) {
  const { t } = useTranslation("ask");
  const { colors } = useTheme();
  const me = useMe();
  const { data: box, isPending, isError } = useBox(handle);
  const answers = useAnswers(box?.owner?.id ?? "");
  if (isPending) return <Spinner />;
  if (isError || !box?.owner)
    return <EmptyState emoji="📪" title={t("box.notFound")} body={t("box.notFoundBody")} />;

  const owner = box.owner;
  const isSelf = owner.id === me?.user?.id;
  const questions = answers.data?.pages.flatMap((page) => page.questions) ?? [];

  return (
    <Page testID="ask-box">
      <Pressable
        accessibilityRole="link"
        onPress={() => router.push(`/u/${owner.handle}`)}
        className="flex-row items-center gap-3"
      >
        <Avatar user={owner} size="lg" outlined />
        <View className="min-w-0 flex-1">
          <Text numberOfLines={1} className="text-xl font-black text-ink">
            {owner.displayName}
          </Text>
          <Text numberOfLines={1} className="font-bold text-ink-soft">
            @{owner.handle}
          </Text>
        </View>
      </Pressable>
      <View className="rounded-card bg-sun px-5 py-4" style={sticker(colors)}>
        <Text
          testID="ask-box-prompt"
          className="text-xl leading-7 font-black"
          style={{ color: INK_ON_BRIGHT }}
        >
          💌 {box.prompt}
        </Text>
      </View>

      <View className="rounded-card bg-surface p-4" style={sticker(colors)}>
        {isSelf ? (
          <View className="gap-3">
            <Text className="font-bold text-ink">{t("box.own")}</Text>
            <ShareBoxButton block />
            <Button
              variant="secondary"
              block
              icon={<Inbox size={18} color={colors.ink} />}
              onPress={() => router.push("/ask/inbox")}
            >
              {t("hub.inbox")}
            </Button>
          </View>
        ) : box.canAsk ? (
          <AskForm box={box} />
        ) : (
          <View testID="ask-cannot" className="items-center gap-3 py-2">
            <Text className="text-4xl leading-[48px]">{box.open ? "🔐" : "😴"}</Text>
            <Text className="text-center font-black text-ink">
              {box.open ? t("box.friendsOnly") : t("box.closed")}
            </Text>
            {box.open && (
              <Button
                variant="secondary"
                size="sm"
                onPress={() => router.push(`/u/${owner.handle}`)}
              >
                {t("box.addFriend")}
              </Button>
            )}
          </View>
        )}
      </View>

      <View className="flex-row items-baseline justify-between px-1 pt-2">
        <Text className="text-lg font-black text-ink">{t("box.answers")}</Text>
        <Text className="text-sm font-bold text-ink-faint">
          {t("box.answersCount", { count: box.answerCount })}
        </Text>
      </View>
      {answers.isPending ? (
        <Spinner />
      ) : questions.length === 0 ? (
        <EmptyState
          emoji="🫧"
          title={t("box.answersEmpty")}
          body={isSelf ? undefined : t("box.answersEmptyBody")}
        />
      ) : (
        <>
          {questions.map((question) => (
            <QaCard key={question.id} question={question} />
          ))}
          <MoreButton query={answers} />
        </>
      )}
    </Page>
  );
}
