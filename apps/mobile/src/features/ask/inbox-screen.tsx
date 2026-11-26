import { useQueryClient } from "@tanstack/react-query";
import { ask } from "@tokuly/api";
import { rememberCounts } from "@tokuly/core/ask";
import { useIsFocused } from "expo-router";
import { MoreHorizontal, PenLine } from "lucide-react-native";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { RefreshControl, ScrollView, Text, View } from "react-native";
import { Button } from "#/components/button";
import { IconButton } from "#/components/icon-button";
import { EmptyState, Spinner } from "#/components/page";
import { Tabs } from "#/components/tabs";
import { sticker, useTheme } from "#/lib/theme";
import { boxAs, stillActive, useInbox, useInboxCounts } from "./data";
import { Age, MoreButton, Picture, QaCard, Sender } from "./parts";
import { useQuestionActions } from "./question-actions";
import { ShareBoxButton } from "./share";

const TABS = ["inbox", "answered", "archived", "filtered"] as const;
type Tab = (typeof TABS)[number];

const FOLDERS: Record<Tab, ask.QuestionFolder> = {
  inbox: ask.QuestionFolder.INBOX,
  answered: ask.QuestionFolder.ANSWERED,
  archived: ask.QuestionFolder.ARCHIVED,
  filtered: ask.QuestionFolder.FILTERED,
};
const EMPTY_EMOJI: Record<Tab, string> = {
  inbox: "📭",
  answered: "✍️",
  archived: "🗄️",
  filtered: "🧹",
};

/** `/ask/inbox`: the owner's new, answered, archived and filtered questions. */
export function InboxScreen() {
  const { t } = useTranslation("ask");
  const queryClient = useQueryClient();
  const [tab, setTab] = useState<Tab>("inbox");
  const counts = useInboxCounts().data;
  const query = useInbox(FOLDERS[tab]);
  const questions = query.data?.pages.flatMap((page) => page.questions) ?? [];

  // Seeing the new questions is reading them: the badge clears, the "未読" marks stay for this visit.
  // Only while the inbox is in front: a question arriving under the answer screen is not seen yet.
  const focused = useIsFocused();
  const marked = useRef(new Set<string>());
  const unreadKey =
    tab === "inbox" && focused
      ? questions.flatMap((question) => (question.read ? [] : [question.id])).join(",")
      : "";
  useEffect(() => {
    const ids = unreadKey
      .split(",")
      .filter((id) => id && !marked.current.has(id))
      .slice(0, 100);
    if (ids.length === 0) return;
    for (const id of ids) marked.current.add(id);
    const { account, client } = boxAs();
    client
      .markQuestionsRead({ questionIds: ids })
      .then((response) => {
        if (stillActive(account)) rememberCounts(queryClient, response.counts);
      })
      .catch(() => {});
  }, [unreadKey, queryClient]);

  // The filter folder only gets a tab while something is held (or it is the one open).
  const tabs = TABS.filter(
    (value) => value !== "filtered" || tab === "filtered" || (counts?.filtered ?? 0) > 0,
  );

  return (
    <View testID="ask-inbox" className="flex-1 bg-bg">
      <View className="px-4 pt-3 pb-2">
        <Tabs
          testID="ask-inbox-tab"
          value={tab}
          onChange={setTab}
          items={tabs.map((value) => ({
            value,
            label: t(`inbox.tabs.${value}`),
            badge:
              value === "inbox"
                ? counts?.unread
                : value === "filtered"
                  ? counts?.filtered
                  : undefined,
          }))}
        />
      </View>
      <ScrollView
        contentContainerClassName="gap-3 px-4 pt-2 pb-12"
        refreshControl={
          <RefreshControl refreshing={query.isRefetching} onRefresh={() => void query.refetch()} />
        }
      >
        {query.isPending ? (
          <Spinner />
        ) : questions.length === 0 ? (
          <EmptyState
            emoji={EMPTY_EMOJI[tab]}
            title={t(`inbox.empty.${tab}`)}
            body={t(`inbox.empty.${tab}Body`)}
            action={tab === "inbox" ? <ShareBoxButton /> : undefined}
          />
        ) : (
          <>
            {questions.map((question) =>
              tab === "answered" ? (
                <AnsweredRow key={question.id} question={question} />
              ) : (
                <QuestionRow key={question.id} question={question} />
              ),
            )}
            <MoreButton query={query} />
          </>
        )}
      </ScrollView>
    </View>
  );
}

/** A question waiting for an answer (or set aside, or held by the filter). */
function QuestionRow({ question }: { question: ask.Question }) {
  const { t } = useTranslation("ask");
  const { colors } = useTheme();
  const actions = useQuestionActions(question);
  const unread = !question.read && question.folder === ask.QuestionFolder.INBOX;
  return (
    <View
      testID="ask-question-row"
      className="gap-2 rounded-card bg-surface px-4 pt-3.5 pb-3"
      style={sticker(colors)}
    >
      <View className="flex-row items-center gap-2">
        <Sender anonymous={question.anonymous} user={question.asker} />
        {unread && (
          <View testID="ask-unread" className="rounded-full bg-pink px-2 py-0.5">
            <Text className="text-[11px] font-black text-white">{t("inbox.unread")}</Text>
          </View>
        )}
        <View className="flex-1" />
        <Age at={question.createdAt} />
      </View>
      <Text className="text-lg leading-6 font-black text-ink">{question.text}</Text>
      <Picture image={question.image} />
      <View className="mt-1 flex-row items-center gap-2">
        <Button
          testID="ask-answer"
          size="sm"
          icon={<PenLine size={16} color={colors["on-brand"]} />}
          onPress={actions.answer}
        >
          {t("inbox.actions.answer")}
        </Button>
        <View className="flex-1" />
        <IconButton
          testID="ask-question-menu"
          label={t("inbox.actions.more")}
          icon={<MoreHorizontal size={20} color={colors["ink-soft"]} />}
          onPress={actions.openMenu}
        />
      </View>
      {actions.sheets}
    </View>
  );
}

/** An answered question, as it looks on the Q&A page, with the owner's menu. */
function AnsweredRow({ question }: { question: ask.Question }) {
  const { t } = useTranslation("ask");
  const { colors } = useTheme();
  const actions = useQuestionActions(question);
  return (
    <>
      <QaCard
        question={question}
        extra={
          <IconButton
            testID="ask-question-menu"
            label={t("inbox.actions.more")}
            icon={<MoreHorizontal size={20} color={colors["ink-soft"]} />}
            onPress={actions.openMenu}
          />
        }
      />
      {actions.sheets}
    </>
  );
}
