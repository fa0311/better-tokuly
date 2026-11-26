import { useQueryClient } from "@tanstack/react-query";
import { ask } from "@tokuly/api";
import { ANSWER_MAX, charCount, dropFromInbox, refreshAnswers } from "@tokuly/core/ask";
import { router } from "expo-router";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Alert, Pressable, Text, TextInput, View } from "react-native";
import { Button } from "#/components/button";
import { EmptyState, Page, Spinner } from "#/components/page";
import { sticker, useTheme } from "#/lib/theme";
import { boxAs, stillActive, useQuestion } from "./data";
import { Counter, Picture, Sender, showAskError } from "./parts";

const AUDIENCES = [
  ["public", ask.AnswerAudience.PUBLIC, "🌏"],
  ["friends", ask.AnswerAudience.FRIENDS, "👥"],
  ["private", ask.AnswerAudience.PRIVATE, "🔒"],
] as const;

/** `/ask/answer/<id>`: answering a question of one's own box, or editing that answer. */
export function AnswerScreen({ questionId }: { questionId: string }) {
  const { t } = useTranslation("ask");
  const { data: question, isPending, isError } = useQuestion(questionId);
  if (isPending) return <Spinner />;
  if (isError || !question)
    return (
      <EmptyState emoji="🫥" title={t("question.notFound")} body={t("question.notFoundBody")} />
    );
  return <AnswerForm question={question} />;
}

/**
 * Text and who may read it. A picture or stamp already on the answer stays
 * as it is (attaching new ones is the web's for now).
 */
function AnswerForm({ question }: { question: ask.Question }) {
  const { t } = useTranslation("ask");
  const { t: tm } = useTranslation("mobile");
  const { colors } = useTheme();
  const queryClient = useQueryClient();
  const existing = question.answer;
  const [text, setText] = useState(existing?.text ?? "");
  const [audience, setAudience] = useState(existing?.audience ?? ask.AnswerAudience.PUBLIC);
  const [sending, setSending] = useState(false);
  const attached = Boolean(existing?.image || existing?.stamp);
  const length = charCount(text.trim());
  const ready = (length > 0 || attached) && length <= ANSWER_MAX;
  const hint = AUDIENCES.find(([, value]) => value === audience)?.[0] ?? "public";

  const submit = async () => {
    const { account, client } = boxAs();
    const body = {
      questionId: question.id,
      text: text.trim(),
      // Unset keeps the picture that is there; the stamp is sent as it is.
      stampId: existing?.stamp?.id ?? "",
      audience,
    };
    setSending(true);
    try {
      const answered = existing
        ? (await client.updateAnswer(body)).question
        : (await client.answerQuestion({ ...body, imageAssetId: "" })).question;
      if (!stillActive(account)) return;
      if (!existing) dropFromInbox(queryClient, question.id);
      refreshAnswers(queryClient, answered);
      Alert.alert(existing ? t("answer.saved") : t("answer.done"));
      router.back();
    } catch (error) {
      if (stillActive(account)) showAskError(error);
    } finally {
      setSending(false);
    }
  };

  return (
    <Page testID="ask-answer-screen">
      <View className="gap-1 rounded-2xl bg-brand-soft px-4 py-3">
        <Sender anonymous={question.anonymous} user={question.asker} />
        <Text className="text-base leading-6 font-black text-ink">{question.text}</Text>
        <Picture image={question.image} />
      </View>
      <View className="rounded-control border-2 border-line bg-surface px-4 pt-3 pb-2">
        <TextInput
          testID="ask-answer-input"
          value={text}
          onChangeText={setText}
          placeholder={t("answer.placeholder")}
          accessibilityLabel={t("answer.placeholder")}
          placeholderTextColor={colors["ink-faint"]}
          multiline
          autoFocus
          textAlignVertical="top"
          className="min-h-28 text-base leading-6 text-ink"
        />
        <View className="items-end">
          <Counter text={text.trim()} max={ANSWER_MAX} />
        </View>
      </View>
      {attached && <Text className="px-1 text-xs text-ink-soft">{tm("ask.keepsAttachment")}</Text>}
      <View className="gap-1.5">
        <Text className="px-1 text-sm font-bold text-ink-soft">{t("answer.audience")}</Text>
        <View
          accessibilityRole="radiogroup"
          className="flex-row gap-1 rounded-full bg-surface-3 p-1"
        >
          {AUDIENCES.map(([key, value, emoji]) => {
            const active = value === audience;
            return (
              <Pressable
                key={key}
                testID={`ask-audience-${key}`}
                accessibilityRole="radio"
                accessibilityState={{ checked: active }}
                onPress={() => setAudience(value)}
                className={`h-9 flex-1 flex-row items-center justify-center gap-1 rounded-full ${active ? "bg-surface" : ""}`}
                style={
                  active ? sticker(colors, "sm") : { borderWidth: 2, borderColor: "transparent" }
                }
              >
                <Text className="text-sm">{emoji}</Text>
                <Text
                  numberOfLines={1}
                  className={`text-sm font-black ${active ? "text-ink" : "text-ink-soft"}`}
                >
                  {t(`answer.audiences.${key}`)}
                </Text>
              </Pressable>
            );
          })}
        </View>
        <Text className="px-2 text-xs text-ink-soft">{t(`answer.audiences.${hint}Hint`)}</Text>
      </View>
      <Button
        testID="ask-answer-submit"
        block
        size="lg"
        loading={sending}
        disabled={!ready}
        onPress={() => void submit()}
      >
        {existing ? t("answer.save") : t("answer.submit")}
      </Button>
    </Page>
  );
}
