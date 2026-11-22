import { Link, useNavigate } from "@tanstack/react-router";
import { ask, common, create } from "@tokuly/api";
import { MoreHorizontal, PenLine } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Button } from "#/components/ui/button";
import { Menu } from "#/components/ui/menu";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { CommentThread } from "#/features/comments";
import { ReportButton } from "#/features/engagement";
import { useMe } from "#/stores/session";
import { useQuestion } from "./api";
import { hasEngagement, QaCard, questionSubject } from "./qa-card";
import { useQuestionActions } from "./question-actions";

/** One question with its answer and the comments on it: where notifications, search and shared links lead. */
export function QuestionScreen({ questionId, focusCommentId }: { questionId: string; focusCommentId?: string }) {
  const { t } = useTranslation("ask");
  const { data: question, isPending, isError } = useQuestion(questionId);

  if (isPending) {
    return (
      <Page title={t("question.title")} back="/ask">
        <Skeleton className="h-48" />
      </Page>
    );
  }
  if (isError || !question) {
    return (
      <Page title={t("question.title")} back="/ask">
        <EmptyState emoji="🫥" title={t("question.notFound")} body={t("question.notFoundBody")} />
      </Page>
    );
  }
  return <QuestionView question={question} focusCommentId={focusCommentId} />;
}

function QuestionView({ question, focusCommentId }: { question: ask.Question; focusCommentId?: string }) {
  const { t } = useTranslation("ask");
  const me = useMe();
  const navigate = useNavigate();
  const owner = question.owner;
  const isOwner = Boolean(owner && owner.id === me?.user?.id);
  const actions = useQuestionActions(question, { onDeleted: () => void navigate({ to: "/ask/inbox", replace: true }) });
  const answer = question.answer;

  return (
    <Page title={t("question.title")} back={isOwner ? "/ask/inbox" : question.sentByMe ? "/ask/sent" : "/ask"}>
      {question.sentByMe && (
        <p className="mb-3 inline-flex rounded-full bg-lime px-3 py-1 text-sm font-black text-[#17122b] sticker-sm" data-testid="sent-by-me">
          💌 {t("question.yours")}
        </p>
      )}
      <QaCard
        question={question}
        link={false}
        extra={
          isOwner ? (
            <Menu
              trigger={
                <Button size="icon-sm" variant="ghost" aria-label={t("inbox.actions.more")}>
                  <MoreHorizontal className="size-5" />
                </Button>
              }
              actions={actions.menu}
            />
          ) : undefined
        }
      />

      {!answer &&
        (isOwner ? (
          <div className="mt-4 flex items-center gap-2">
            <Button icon={<PenLine className="size-4" />} onClick={actions.answer}>
              {t("inbox.actions.answer")}
            </Button>
            <Menu
              trigger={
                <Button size="icon" variant="ghost" aria-label={t("inbox.actions.more")}>
                  <MoreHorizontal className="size-5" />
                </Button>
              }
              actions={actions.menu}
            />
          </div>
        ) : (
          <EmptyState emoji="⏳" title={t("question.waiting")} body={t("question.waitingBody")} />
        ))}
      {answer?.audience === ask.AnswerAudience.PRIVATE && question.sentByMe && owner && (
        <p className="mt-3 rounded-2xl bg-surface-2 px-4 py-3 text-sm font-bold text-ink-soft">🔒 {t("question.privateNote", { name: owner.displayName })}</p>
      )}

      {!isOwner && owner && (
        <div className="mt-4 flex flex-wrap items-center gap-2">
          <Link to="/ask/u/$handle" params={{ handle: owner.handle }} className="pressable sticker inline-flex h-11 items-center gap-2 rounded-full bg-surface px-5 font-bold">
            💌 {t("question.askToo", { name: owner.displayName })}
          </Link>
          {answer && <ReportButton subject={questionSubject(question.id)} label={question.text} />}
        </div>
      )}

      {hasEngagement(question) && (
        <section className="mt-6">
          <CommentThread subject={questionSubject(question.id)} subjectAuthor={owner ? create(common.AuthorSchema, { user: owner }) : undefined} focusCommentId={focusCommentId} />
        </section>
      )}
      {isOwner && actions.dialogs}
    </Page>
  );
}
