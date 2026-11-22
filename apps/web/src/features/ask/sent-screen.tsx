import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { ask, toMillis } from "@tokuly/api";
import { ChevronRight } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { compactAge } from "#/lib/format";
import { askKeys, boxClient, useAskedQuestions } from "./api";
import { Picture, toastError } from "./parts";

/** The asker's side: every question they sent, waiting or answered. */
export function SentScreen() {
  const { t } = useTranslation("ask");
  const query = useAskedQuestions();
  const questions = query.data?.pages.flatMap((page) => page.questions) ?? [];

  return (
    <Page title={t("sent.title")} back="/ask">
      {query.isPending ? (
        <div className="flex flex-col gap-3">
          <Skeleton className="h-24" />
          <Skeleton className="h-24" />
        </div>
      ) : questions.length === 0 ? (
        <EmptyState emoji="✉️" title={t("sent.empty")} body={t("sent.emptyBody")} />
      ) : (
        <div className="flex flex-col gap-3">
          {questions.map((question) => (
            <SentRow key={question.id} question={question} />
          ))}
          <LoadMore query={query} />
        </div>
      )}
    </Page>
  );
}

function SentRow({ question }: { question: ask.Question }) {
  const { t } = useTranslation("ask");
  const queryClient = useQueryClient();
  const [confirming, setConfirming] = useState(false);
  const answered = question.sentState === ask.SentState.ANSWERED;
  const owner = question.owner;

  const withdraw = useMutation({
    mutationFn: () => boxClient().withdrawQuestion({ questionId: question.id }),
    onSuccess: () => {
      toast.success(t("sent.withdrawn"));
      setConfirming(false);
    },
    onError: toastError,
    // Also after a failure: "already answered" means the list is out of date.
    onSettled: () => queryClient.invalidateQueries({ queryKey: askKeys.asked }),
  });

  return (
    <article className="sticker rounded-card bg-surface px-4 pt-3.5 pb-3" data-testid="sent-row">
      <div className="flex items-center gap-2">
        {owner && (
          <Link to="/ask/u/$handle" params={{ handle: owner.handle }} className="inline-flex min-w-0 items-center gap-1.5 text-sm font-black">
            <Avatar user={owner} size="xs" />
            <span className="truncate">{t("sent.to", { name: owner.displayName })}</span>
          </Link>
        )}
        {!question.anonymous && <span className="rounded-full bg-surface-2 px-2 py-0.5 text-[11px] font-black text-ink-soft">🙋 {t("sent.signed")}</span>}
        <span className="flex-1" />
        <time className="text-xs font-bold text-ink-faint">{compactAge(toMillis(question.createdAt))}</time>
      </div>
      <p className="mt-2 leading-snug font-black break-words whitespace-pre-wrap">{question.text}</p>
      {question.image && <Picture image={question.image} className="mt-2 max-w-40" />}
      <div className="mt-3 flex items-center gap-2">
        {answered ? (
          <Link
            to="/ask/q/$questionId"
            params={{ questionId: question.id }}
            className="pressable sticker-sm inline-flex h-9 items-center gap-1 rounded-full bg-lime pr-2 pl-4 text-sm font-black text-[#17122b]"
            data-testid="sent-state"
          >
            🎉 {t("sent.answered")}
            <ChevronRight className="size-4" />
          </Link>
        ) : (
          <>
            <span className="inline-flex h-9 items-center rounded-full bg-surface-2 px-4 text-sm font-bold text-ink-soft" data-testid="sent-state">
              ⏳ {t("sent.waiting")}
            </span>
            <span className="flex-1" />
            <Button size="sm" variant="ghost" className="text-ink-soft" onClick={() => setConfirming(true)}>
              {t("sent.withdraw")}
            </Button>
          </>
        )}
      </div>
      <ConfirmDialog
        open={confirming}
        onOpenChange={setConfirming}
        title={t("sent.confirmWithdraw.title")}
        description={t("sent.confirmWithdraw.body")}
        confirmLabel={t("sent.confirmWithdraw.confirm")}
        danger
        loading={withdraw.isPending}
        onConfirm={() => withdraw.mutate()}
      />
    </article>
  );
}
