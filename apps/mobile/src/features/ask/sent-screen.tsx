import { useQueryClient } from "@tanstack/react-query";
import { ask } from "@tokuly/api";
import { askKeys } from "@tokuly/core/ask";
import { router } from "expo-router";
import { useTranslation } from "react-i18next";
import { Alert, Pressable, RefreshControl, ScrollView, Text, View } from "react-native";
import { Avatar } from "#/components/avatar";
import { Button } from "#/components/button";
import { EmptyState, Spinner } from "#/components/page";
import { sticker, useTheme } from "#/lib/theme";
import { boxAs, stillActive, useAskedQuestions } from "./data";
import { Age, MoreButton, Picture, showAskError } from "./parts";

/**
 * `/ask/sent`: the asker's side. Each question is "waiting" or "answered",
 * nothing else: whatever else became of it is the owner's business.
 */
export function SentScreen() {
  const { t } = useTranslation("ask");
  const query = useAskedQuestions();
  const questions = query.data?.pages.flatMap((page) => page.questions) ?? [];
  return (
    <ScrollView
      testID="ask-sent-list"
      className="flex-1 bg-bg"
      contentContainerClassName="gap-3 px-4 pt-4 pb-12"
      refreshControl={
        <RefreshControl refreshing={query.isRefetching} onRefresh={() => void query.refetch()} />
      }
    >
      {query.isPending ? (
        <Spinner />
      ) : questions.length === 0 ? (
        <EmptyState emoji="✉️" title={t("sent.empty")} body={t("sent.emptyBody")} />
      ) : (
        <>
          {questions.map((question) => (
            <SentRow key={question.id} question={question} />
          ))}
          <MoreButton query={query} />
        </>
      )}
    </ScrollView>
  );
}

function SentRow({ question }: { question: ask.Question }) {
  const { t } = useTranslation("ask");
  const { t: tc } = useTranslation("common");
  const { colors } = useTheme();
  const queryClient = useQueryClient();
  const answered = question.sentState === ask.SentState.ANSWERED;
  const owner = question.owner;

  const withdraw = () =>
    Alert.alert(t("sent.confirmWithdraw.title"), t("sent.confirmWithdraw.body"), [
      { text: tc("actions.cancel"), style: "cancel" },
      {
        text: t("sent.confirmWithdraw.confirm"),
        style: "destructive",
        onPress: async () => {
          const { account, client } = boxAs();
          try {
            await client.withdrawQuestion({ questionId: question.id });
            if (stillActive(account)) Alert.alert(t("sent.withdrawn"));
          } catch (error) {
            if (stillActive(account)) showAskError(error);
          }
          // Also after a failure: "already answered" means the list is out of date.
          if (stillActive(account)) void queryClient.invalidateQueries({ queryKey: askKeys.asked });
        },
      },
    ]);

  return (
    <View
      testID="ask-sent-row"
      className="gap-2 rounded-card bg-surface px-4 pt-3.5 pb-3"
      style={sticker(colors)}
    >
      <View className="flex-row items-center gap-2">
        {owner && (
          <Pressable
            accessibilityRole="link"
            onPress={() => router.push(`/ask/u/${owner.handle}`)}
            className="min-w-0 shrink flex-row items-center gap-1.5"
          >
            <Avatar user={owner} size="xs" />
            <Text numberOfLines={1} className="shrink text-sm font-black text-ink">
              {t("sent.to", { name: owner.displayName })}
            </Text>
          </Pressable>
        )}
        {!question.anonymous && (
          <View className="rounded-full bg-surface-2 px-2 py-0.5">
            <Text className="text-[11px] font-black text-ink-soft">🙋 {t("sent.signed")}</Text>
          </View>
        )}
        <View className="flex-1" />
        <Age at={question.createdAt} />
      </View>
      <Text className="text-base leading-6 font-black text-ink">{question.text}</Text>
      <Picture image={question.image} />
      <View className="mt-1 flex-row items-center gap-2">
        {answered ? (
          <Button
            testID="ask-sent-state"
            variant="lime"
            size="sm"
            onPress={() => router.push(`/ask/q/${question.id}`)}
            icon={<Text>🎉</Text>}
          >
            {t("sent.answered")}
          </Button>
        ) : (
          <>
            <Text testID="ask-sent-state" className="text-sm font-black text-ink-soft">
              ⏳ {t("sent.waiting")}
            </Text>
            <View className="flex-1" />
            <Button testID="ask-withdraw" variant="ghost" size="sm" onPress={withdraw}>
              {t("sent.withdraw")}
            </Button>
          </>
        )}
      </View>
    </View>
  );
}
