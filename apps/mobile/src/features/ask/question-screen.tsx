import { ask } from "@tokuly/api";
import { router } from "expo-router";
import { Flag, MoreHorizontal, PenLine } from "lucide-react-native";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Text, View } from "react-native";
import { Button } from "#/components/button";
import { IconButton } from "#/components/icon-button";
import { EmptyState, Page, Spinner } from "#/components/page";
import { useMe } from "#/features/session/store";
import { INK_ON_BRIGHT, useTheme } from "#/lib/theme";
import { useQuestion } from "./data";
import { QaCard, ReportSheet } from "./parts";
import { useQuestionActions } from "./question-actions";

/** `/ask/q/<id>`: one question with its answer, where notifications and links lead. */
export function QuestionScreen({ questionId }: { questionId: string }) {
  const { t } = useTranslation("ask");
  const { data: question, isPending, isError } = useQuestion(questionId);
  if (isPending) return <Spinner />;
  if (isError || !question)
    return (
      <EmptyState emoji="🫥" title={t("question.notFound")} body={t("question.notFoundBody")} />
    );
  return <QuestionView question={question} />;
}

function QuestionView({ question }: { question: ask.Question }) {
  const { t } = useTranslation("ask");
  const { colors } = useTheme();
  const me = useMe();
  const owner = question.owner;
  const isOwner = Boolean(owner && owner.id === me?.user?.id);
  const actions = useQuestionActions(question, {
    onDeleted: () => (router.canGoBack() ? router.back() : router.replace("/ask/inbox")),
  });
  const [reporting, setReporting] = useState(false);
  const answer = question.answer;
  const menu = isOwner ? (
    <IconButton
      testID="ask-question-menu"
      label={t("inbox.actions.more")}
      icon={<MoreHorizontal size={20} color={colors["ink-soft"]} />}
      onPress={actions.openMenu}
    />
  ) : undefined;

  return (
    <Page testID="ask-question">
      {question.sentByMe && (
        <View testID="ask-sent-by-me" className="self-start rounded-full bg-lime px-3 py-1">
          <Text className="text-sm font-black" style={{ color: INK_ON_BRIGHT }}>
            💌 {t("question.yours")}
          </Text>
        </View>
      )}
      <QaCard question={question} link={false} extra={menu} />

      {!answer &&
        (isOwner ? (
          <View className="flex-row items-center gap-2">
            <Button
              testID="ask-answer"
              icon={<PenLine size={18} color={colors["on-brand"]} />}
              onPress={actions.answer}
            >
              {t("inbox.actions.answer")}
            </Button>
            {menu}
          </View>
        ) : (
          <EmptyState emoji="⏳" title={t("question.waiting")} body={t("question.waitingBody")} />
        ))}
      {answer?.audience === ask.AnswerAudience.PRIVATE && question.sentByMe && owner && (
        <Text className="rounded-2xl bg-surface-2 px-4 py-3 text-sm font-bold text-ink-soft">
          🔒 {t("question.privateNote", { name: owner.displayName })}
        </Text>
      )}

      {!isOwner && owner && (
        <View className="flex-row flex-wrap items-center gap-2">
          <Button
            testID="ask-ask-too"
            variant="secondary"
            onPress={() => router.push(`/ask/u/${owner.handle}`)}
            icon={<Text>💌</Text>}
          >
            {t("question.askToo", { name: owner.displayName })}
          </Button>
          {answer && (
            <Button
              testID="ask-report"
              variant="ghost"
              icon={<Flag size={16} color={colors.ink} />}
              onPress={() => setReporting(true)}
            >
              {t("inbox.actions.report")}
            </Button>
          )}
        </View>
      )}
      {isOwner && actions.sheets}
      <ReportSheet
        questionId={reporting ? question.id : null}
        onClose={() => setReporting(false)}
      />
    </Page>
  );
}
