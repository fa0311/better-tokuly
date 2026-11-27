import { useQueryClient } from "@tanstack/react-query";
import { ask, toMillis } from "@tokuly/api";
import { rememberCounts } from "@tokuly/core/ask";
import { Archive, ArchiveRestore, MoreHorizontal, PenLine, Share2 } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "#/components/ui/button";
import { LoadMore } from "#/components/ui/load-more";
import { Menu } from "#/components/ui/menu";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { Tabs } from "#/components/ui/tabs";
import { compactAge } from "#/lib/format";
import { useMe } from "#/stores/session";
import { boxClient, useInbox, useInboxCounts, useSettings } from "./api";
import { INBOX_TABS, type InboxTab } from "./inbox-search";
import { Picture, Sender } from "./parts";
import { QaCard } from "./qa-card";
import { useQuestionActions } from "./question-actions";
import { BoxShareSheet } from "./share-sheet";
import { SwipeRow } from "./swipe-row";

const FOLDERS: Record<InboxTab, ask.QuestionFolder> = {
  inbox: ask.QuestionFolder.INBOX,
  answered: ask.QuestionFolder.ANSWERED,
  archived: ask.QuestionFolder.ARCHIVED,
  filtered: ask.QuestionFolder.FILTERED,
};
const EMPTY_EMOJI: Record<InboxTab, string> = {
  inbox: "📭",
  answered: "✍️",
  archived: "🗄️",
  filtered: "🧹",
};

/** The owner's inbox: new, answered, archived and filtered questions. */
export function InboxScreen({
  tab,
  onTabChange,
}: {
  tab: InboxTab;
  onTabChange(tab: InboxTab): void;
}) {
  const { t } = useTranslation("ask");
  const queryClient = useQueryClient();
  const me = useMe();
  const settings = useSettings();
  const counts = useInboxCounts().data;
  const query = useInbox(FOLDERS[tab]);
  const [sharing, setSharing] = useState(false);
  const questions = query.data?.pages.flatMap((page) => page.questions) ?? [];

  // Seeing the new questions is reading them: the badge clears, the dots stay for this visit.
  const marked = useRef(new Set<string>());
  const unreadKey =
    tab === "inbox"
      ? questions.flatMap((question) => (question.read ? [] : [question.id])).join(",")
      : "";
  useEffect(() => {
    const ids = unreadKey
      .split(",")
      .filter((id) => id && !marked.current.has(id))
      .slice(0, 100);
    if (ids.length === 0) return;
    for (const id of ids) marked.current.add(id);
    boxClient()
      .markQuestionsRead({ questionIds: ids })
      .then((response) => rememberCounts(queryClient, response.counts))
      .catch(() => {});
  }, [unreadKey, queryClient]);

  // The filter folder only gets a tab while something is held (or it is the one open).
  const tabs = INBOX_TABS.filter(
    (value) => value !== "filtered" || tab === "filtered" || (counts?.filtered ?? 0) > 0,
  );

  return (
    <Page
      title={t("inbox.title")}
      back="/ask"
      actions={
        <Button
          size="icon-sm"
          variant="ghost"
          aria-label={t("inbox.share")}
          onClick={() => setSharing(true)}
        >
          <Share2 className="size-5" />
        </Button>
      }
    >
      <Tabs
        value={tab}
        onChange={onTabChange}
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

      <div className="mt-4 flex flex-col gap-3" data-testid="inbox-list">
        {query.isPending ? (
          <>
            <Skeleton className="h-28" />
            <Skeleton className="h-28" />
            <Skeleton className="h-28" />
          </>
        ) : questions.length === 0 ? (
          <EmptyState
            emoji={EMPTY_EMOJI[tab]}
            title={t(`inbox.empty.${tab}`)}
            body={t(`inbox.empty.${tab}Body`)}
            action={
              tab === "inbox" ? (
                <Button icon={<Share2 className="size-4" />} onClick={() => setSharing(true)}>
                  {t("inbox.share")}
                </Button>
              ) : undefined
            }
          />
        ) : (
          <>
            {tab !== "answered" && tab !== "filtered" && (
              <p className="text-center text-xs font-bold text-ink-faint md:hidden">
                {tab === "inbox" ? t("inbox.swipeHint") : t("inbox.swipeHintArchived")}
              </p>
            )}
            {questions.map((question) =>
              tab === "answered" ? (
                <AnsweredRow key={question.id} question={question} />
              ) : (
                <QuestionRow key={question.id} question={question} />
              ),
            )}
            <LoadMore query={query} />
          </>
        )}
      </div>

      {sharing && me?.user && (
        <BoxShareSheet
          open
          onOpenChange={setSharing}
          handle={me.user.handle}
          prompt={settings.data?.boxPrompt ?? ""}
        />
      )}
    </Page>
  );
}

/** A question waiting for an answer (or set aside, or held by the filter). */
function QuestionRow({ question }: { question: ask.Question }) {
  const { t } = useTranslation("ask");
  const actions = useQuestionActions(question);
  const archived = question.folder === ask.QuestionFolder.ARCHIVED;
  const filtered = question.folder === ask.QuestionFolder.FILTERED;

  return (
    <>
      <SwipeRow
        className="sticker"
        toLeft={
          filtered
            ? undefined
            : {
                label: archived ? t("inbox.actions.unarchive") : t("inbox.actions.archive"),
                icon: archived ? (
                  <ArchiveRestore className="size-5" />
                ) : (
                  <Archive className="size-5" />
                ),
                tint: "bg-sky",
                onCommit: actions.toggleArchive,
              }
        }
        toRight={{
          label: t("inbox.actions.answer"),
          icon: <PenLine className="size-5" />,
          tint: "bg-lime",
          onCommit: actions.answer,
        }}
      >
        <article className="bg-surface px-4 pt-3.5 pb-3" data-testid="question-row">
          <div className="flex items-center gap-2">
            <Sender anonymous={question.anonymous} user={question.asker} />
            {!question.read && question.folder === ask.QuestionFolder.INBOX && (
              <span
                className="rounded-full bg-pink px-2 py-0.5 text-[11px] font-black text-white"
                data-testid="unread"
              >
                {t("inbox.unread")}
              </span>
            )}
            <span className="flex-1" />
            <time className="text-xs font-bold text-ink-faint">
              {compactAge(toMillis(question.createdAt))}
            </time>
          </div>
          <p className="mt-2 text-lg leading-snug font-black break-words whitespace-pre-wrap">
            {question.text}
          </p>
          {question.image && <Picture image={question.image} className="mt-2 max-w-xs" />}
          <div className="mt-3 flex items-center gap-2">
            <Button size="sm" icon={<PenLine className="size-4" />} onClick={actions.answer}>
              {t("inbox.actions.answer")}
            </Button>
            <span className="flex-1" />
            <Menu
              trigger={
                <Button size="icon-sm" variant="ghost" aria-label={t("inbox.actions.more")}>
                  <MoreHorizontal className="size-5" />
                </Button>
              }
              actions={actions.menu}
            />
          </div>
        </article>
      </SwipeRow>
      {actions.dialogs}
    </>
  );
}

/** An answered question, as it looks on the Q&A page, with the owner's menu. */
function AnsweredRow({ question }: { question: ask.Question }) {
  const { t } = useTranslation("ask");
  const actions = useQuestionActions(question);
  return (
    <>
      <QaCard
        question={question}
        extra={
          <Menu
            trigger={
              <Button size="icon-sm" variant="ghost" aria-label={t("inbox.actions.more")}>
                <MoreHorizontal className="size-5" />
              </Button>
            }
            actions={actions.menu}
          />
        }
      />
      {actions.dialogs}
    </>
  );
}
