import { Link } from "@tanstack/react-router";
import { ask, common, toMillis } from "@tokuly/api";
import { Heart, ImageDown, MessageCircle } from "lucide-react";
import { type ReactNode, useState } from "react";
import { useTranslation } from "react-i18next";
import { Avatar } from "#/components/ui/avatar";
import { compactCount, LikeButton, type Subject } from "#/features/engagement";
import { cn } from "#/lib/cn";
import { compactAge } from "#/lib/format";
import { AnswerBody, AudienceBadge, Picture, Sender } from "./parts";
import { AnswerShareSheet } from "./share-sheet";

export const questionSubject = (id: string): Subject => ({ type: common.SubjectType.QUESTION, id });

/** Whether likes and comments exist for this answer: only on the Q&A page (public or friends). */
export const hasEngagement = (question: ask.Question): boolean =>
  question.answer?.audience === ask.AnswerAudience.PUBLIC || question.answer?.audience === ask.AnswerAudience.FRIENDS;

/**
 * One answered question as a card: the question on top (who asked, when
 * they signed), the owner's answer below, and — for people who are signed in —
 * the like, the comments and "share as an image".
 */
export function QaCard({
  question,
  guest,
  link = true,
  highlighted,
  extra,
}: {
  question: ask.Question;
  /** Signed-out view: counts instead of controls, no links into the app. */
  guest?: boolean;
  /** The question and the comment count lead to the question's page. */
  link?: boolean;
  highlighted?: boolean;
  /** More controls at the end of the footer (the owner's menu). */
  extra?: ReactNode;
}) {
  const { t, i18n } = useTranslation("ask");
  const [sharing, setSharing] = useState(false);
  const answer = question.answer;
  const engaged = hasEngagement(question);
  const text = <p className="mt-1.5 text-lg leading-snug font-black break-words whitespace-pre-wrap">{question.text}</p>;

  return (
    <article className={cn("sticker overflow-hidden rounded-card bg-surface", highlighted && "ring-4 ring-lime")} data-testid="qa-card">
      <div className="bg-brand-soft px-4 pt-3.5 pb-4">
        <div className="flex items-center gap-2">
          {guest ? <GuestSender question={question} /> : <Sender anonymous={question.anonymous} user={question.asker} />}
          <span className="flex-1" />
          <time className="text-xs font-bold text-ink-soft">{compactAge(toMillis(question.createdAt))}</time>
        </div>
        {link && !guest ? (
          <Link to="/ask/q/$questionId" params={{ questionId: question.id }} className="block">
            {text}
          </Link>
        ) : (
          text
        )}
        {question.image && <Picture image={question.image} className="mt-2 max-w-xs" />}
      </div>
      {answer && (
        <div className="flex gap-3 border-t-2 border-ink px-4 pt-3.5 pb-3">
          <Avatar user={question.owner} size="sm" />
          <div className="min-w-0 flex-1">
            <AnswerBody answer={answer} />
            <div className="mt-2.5 flex min-h-9 flex-wrap items-center gap-x-1 gap-y-1.5 text-ink-soft">
              {engaged &&
                (guest ? (
                  <>
                    <span className="inline-flex items-center gap-1 pr-3 text-sm font-bold" title={t("question.like", { count: question.likeCount })}>
                      <Heart className="size-4" />
                      {compactCount(question.likeCount, i18n.language)}
                    </span>
                    <span className="inline-flex items-center gap-1 pr-3 text-sm font-bold" title={t("question.commentCount", { count: question.commentCount })}>
                      <MessageCircle className="size-4" />
                      {compactCount(question.commentCount, i18n.language)}
                    </span>
                  </>
                ) : (
                  <>
                    <LikeButton subject={questionSubject(question.id)} />
                    <Link
                      to="/ask/q/$questionId"
                      params={{ questionId: question.id }}
                      aria-label={t("question.commentCount", { count: question.commentCount })}
                      className="inline-flex h-9 items-center gap-1.5 rounded-full px-3 text-sm font-bold hover:bg-surface-2 hover:text-ink"
                    >
                      <MessageCircle className="size-[18px]" />
                      {question.commentCount > 0 && compactCount(question.commentCount, i18n.language)}
                    </Link>
                  </>
                ))}
              {!guest && answer.audience === ask.AnswerAudience.PUBLIC && (
                <button
                  type="button"
                  aria-label={t("inbox.actions.shareAnswer")}
                  title={t("inbox.actions.shareAnswer")}
                  onClick={() => setSharing(true)}
                  className="grid size-9 place-items-center rounded-full hover:bg-surface-2 hover:text-ink"
                >
                  <ImageDown className="size-[18px]" />
                </button>
              )}
              <AudienceBadge audience={answer.audience} />
              <span className="flex-1" />
              {answer.editedAt && <span className="text-xs font-bold text-ink-faint">{t("answer.edited")}</span>}
              {extra}
            </div>
          </div>
        </div>
      )}
      {sharing && <AnswerShareSheet open onOpenChange={setSharing} question={question} />}
    </article>
  );
}

/** Who asked, without a link into the app (the public page). */
function GuestSender({ question }: { question: ask.Question }) {
  const { t } = useTranslation("ask");
  return (
    <span className="inline-flex min-w-0 items-center gap-1.5 text-sm font-black text-ink-soft" data-testid="sender">
      {!question.anonymous && question.asker ? (
        <>
          <Avatar user={question.asker} size="xs" />
          <span className="truncate text-ink">{question.asker.displayName}</span>
        </>
      ) : (
        <>
          <span aria-hidden className="grid size-6 place-items-center rounded-[0.45rem] bg-surface-3 text-xs">
            🙈
          </span>
          {t("anonymous")}
        </>
      )}
    </span>
  );
}
