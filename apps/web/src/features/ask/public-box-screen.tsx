import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { Link, Navigate, useNavigate } from "@tanstack/react-router";
import { PAGE_SIZE, QUESTION_MAX } from "@tokuly/core/ask";
import { useState } from "react";
import { Button } from "#/components/ui/button";
import { Card } from "#/components/ui/card";
import { LoadMore } from "#/components/ui/load-more";
import { Logo } from "#/components/ui/logo";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { useTranslation } from "#/lib/use-translation";
import { useSessionStatus } from "#/stores/session";
import { boxClient } from "./api";
import { BoxHeader } from "./box-screen";
import { rememberPendingAsk } from "./guest-return";
import { Counter, SectionTitle } from "./parts";
import { QaCard } from "./qa-card";

/**
 * `/q/<handle>`: the page a shared question-box link opens. No account is
 * needed to see the box and its public answers; asking leads through sign-up
 * (or sign-in) and comes back to the box with the question still written.
 * Someone who is signed in is taken straight to the box inside the app.
 */
export function PublicBoxScreen({ handle, answerId }: { handle: string; answerId?: string }) {
  const { t } = useTranslation("ask");
  const status = useSessionStatus();
  const navigate = useNavigate();
  const [text, setText] = useState("");
  const guest = status === "signedOut";

  // Guest reads are cached apart from the signed-in ones: the answers differ.
  const box = useQuery({
    queryKey: ["ask", "guest", "box", handle],
    queryFn: async () => (await boxClient().getBox({ user: handle })).box ?? null,
    enabled: guest,
  });
  const answers = useInfiniteQuery({
    queryKey: ["ask", "guest", "answers", handle],
    queryFn: ({ pageParam }) =>
      boxClient().listAnswers({ user: handle, page: { cursor: pageParam, limit: PAGE_SIZE } }),
    initialPageParam: "",
    getNextPageParam: (page) => (page.page?.hasMore ? page.page.nextCursor : undefined),
    enabled: guest,
  });
  // A shared answer (`?a=<id>`) is shown first.
  const shared = useQuery({
    queryKey: ["ask", "guest", "question", answerId],
    queryFn: async () =>
      (await boxClient().getQuestion({ questionId: answerId! })).question ?? null,
    enabled: guest && Boolean(answerId),
    retry: false,
  });

  if (status === "signedIn") {
    return answerId ? (
      <Navigate to="/ask/q/$questionId" params={{ questionId: answerId }} replace />
    ) : (
      <Navigate to="/ask/u/$handle" params={{ handle }} replace />
    );
  }

  const toAccount = (to: "/signup" | "/login") => {
    rememberPendingAsk({ handle, text });
    // Either way leads back to this box, where the draft is waiting.
    void navigate({ to, search: { redirect: `/ask/u/${handle}` } });
  };
  const owner = box.data?.owner;
  const sharedQuestion =
    shared.data?.answer && shared.data.owner?.id === owner?.id ? shared.data : undefined;
  const questions = (answers.data?.pages.flatMap((page) => page.questions) ?? []).filter(
    (question) => question.id !== sharedQuestion?.id,
  );

  return (
    <main className="min-h-dvh bg-bg">
      <header className="mx-auto flex max-w-5xl items-center justify-between px-4 py-4">
        <Link to="/" aria-label="Tokuly">
          <Logo />
        </Link>
        <Link
          to="/login"
          search={{ redirect: `/ask/u/${handle}` }}
          className="pressable sticker-sm rounded-full bg-surface px-5 py-2 text-sm font-bold"
        >
          {t("guest.signInShort")}
        </Link>
      </header>

      <div className="mx-auto max-w-5xl px-4 pb-16">
        {status === "unknown" || box.isPending ? (
          <div className="mx-auto max-w-md">
            <Skeleton className="h-24" />
            <Skeleton className="mt-4 h-56" />
          </div>
        ) : !box.data || !owner ? (
          <EmptyState emoji="📪" title={t("box.notFound")} body={t("box.notFoundBody")} />
        ) : (
          <div className="grid grid-cols-1 items-start gap-x-8 lg:grid-cols-[minmax(0,26rem)_minmax(0,1fr)]">
            <div className="lg:sticky lg:top-6">
              <BoxHeader box={box.data} guest />
              <Card className="mt-5">
                {box.data.open ? (
                  <form
                    className="flex flex-col gap-3"
                    onSubmit={(event) => {
                      event.preventDefault();
                      toAccount("/signup");
                    }}
                  >
                    <p className="font-black">{t("guest.lead", { name: owner.displayName })}</p>
                    <div className="rounded-control border-2 border-line bg-surface transition-colors focus-within:border-brand">
                      <textarea
                        value={text}
                        onChange={(event) => setText(event.target.value)}
                        placeholder={t("askForm.placeholder")}
                        aria-label={t("askForm.placeholder")}
                        rows={3}
                        className="block w-full resize-none bg-transparent px-4 pt-3 text-[15px] leading-relaxed placeholder:text-ink-faint focus:outline-none"
                      />
                      <div className="flex justify-end px-4 pb-2">
                        <Counter text={text.trim()} max={QUESTION_MAX} />
                      </div>
                    </div>
                    <Button type="submit" block size="lg">
                      {t("guest.signUp")}
                    </Button>
                    <Button block variant="secondary" onClick={() => toAccount("/login")}>
                      {t("guest.signIn")}
                    </Button>
                    <p className="px-1 text-xs leading-relaxed text-ink-soft">{t("guest.note")}</p>
                  </form>
                ) : (
                  <div className="flex flex-col items-center gap-2 py-2 text-center">
                    <div className="text-4xl">😴</div>
                    <p className="font-black">{t("box.closed")}</p>
                  </div>
                )}
              </Card>
            </div>

            <section>
              <SectionTitle
                className="lg:mt-0"
                action={
                  <span className="text-sm font-bold text-ink-faint">
                    {t("box.answersCount", { count: box.data.answerCount })}
                  </span>
                }
              >
                {t("box.answers")}
              </SectionTitle>
              {answers.isPending ? (
                <Skeleton className="h-40" />
              ) : questions.length === 0 && !sharedQuestion ? (
                <EmptyState
                  emoji="🫧"
                  title={t("box.answersEmpty")}
                  body={t("box.answersEmptyBody")}
                />
              ) : (
                <div className="flex flex-col gap-4">
                  {sharedQuestion && <QaCard question={sharedQuestion} guest highlighted />}
                  {questions.map((question) => (
                    <QaCard key={question.id} question={question} guest />
                  ))}
                  <LoadMore query={answers} />
                </div>
              )}
            </section>
          </div>
        )}

        <footer className="mt-14 flex flex-col items-center gap-3 text-center">
          <p className="text-sm font-bold text-ink-soft">{t("guest.brand")}</p>
          <Link
            to="/signup"
            className="pressable sticker inline-flex h-11 items-center rounded-full bg-lime px-6 font-black text-[#17122b]"
          >
            {t("guest.makeOwn")}
          </Link>
        </footer>
      </div>
    </main>
  );
}
