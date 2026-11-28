import { Link } from "@tanstack/react-router";
import type { ask } from "@tokuly/api";
import { Inbox, Share2 } from "lucide-react";
import { useState } from "react";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { Card } from "#/components/ui/card";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { EmojiText } from "#/features/emoji";
import { useTranslation } from "#/lib/use-translation";
import { useMe } from "#/stores/session";
import { useAnswers, useBox, usePromptsOf } from "./api";
import { takeDraft } from "./guest-return";
import { SectionTitle } from "./parts";
import { PromptCard } from "./prompt-card";
import { QaCard } from "./qa-card";
import { AskForm } from "./send-form";
import { BoxShareSheet } from "./share-sheet";

/** The head of a box: whose it is and the line they put above it. */
export function BoxHeader({ box, guest }: { box: ask.QuestionBox; guest?: boolean }) {
  const owner = box.owner!;
  const who = (
    <>
      <Avatar user={owner} size="lg" outlined />
      <span className="min-w-0">
        <span className="block truncate text-xl font-black">
          <EmojiText text={owner.displayName} />
        </span>
        <span className="block truncate font-bold text-ink-soft">@{owner.handle}</span>
      </span>
    </>
  );
  return (
    <div>
      {guest ? (
        <div className="flex items-center gap-3">{who}</div>
      ) : (
        <Link to="/u/$handle" params={{ handle: owner.handle }} className="flex items-center gap-3">
          {who}
        </Link>
      )}
      <p
        className="sticker relative mt-4 rounded-card bg-sun px-5 py-4 text-xl leading-snug font-black break-words text-[#17122b]"
        data-testid="box-prompt"
      >
        <span aria-hidden className="mr-1.5">
          💌
        </span>
        {box.prompt}
      </p>
    </div>
  );
}

/** Someone's question box inside the app: ask them something, read what they answered. */
export function BoxScreen({ handle }: { handle: string }) {
  const { t } = useTranslation("ask");
  const me = useMe();
  const { data: box, isPending, isError } = useBox(handle);
  const answers = useAnswers(box?.owner?.id);
  const [draft] = useState(() => takeDraft(handle));
  const [sharing, setSharing] = useState(false);
  const isSelf = Boolean(box?.owner && box.owner.id === me?.user?.id);
  // A friend's open prompts sit next to their box; for anyone else the list is empty.
  const prompts = usePromptsOf(box?.owner?.id ?? "", Boolean(box?.owner) && !isSelf);

  if (isPending) {
    return (
      <Page title={t("title")} back="/ask">
        <Skeleton className="h-24" />
        <Skeleton className="mt-4 h-56" />
      </Page>
    );
  }
  if (isError || !box?.owner) {
    return (
      <Page title={t("title")} back="/ask">
        <EmptyState emoji="📪" title={t("box.notFound")} body={t("box.notFoundBody")} />
      </Page>
    );
  }

  const owner = box.owner;
  const questions = answers.data?.pages.flatMap((page) => page.questions) ?? [];
  const openPrompts =
    prompts.data?.pages.flatMap((page) => page.prompts).filter((prompt) => prompt.open) ?? [];

  return (
    <Page title={t("box.title", { name: owner.displayName })} back="/ask" wide>
      <div className="grid grid-cols-1 items-start gap-x-8 lg:grid-cols-[minmax(0,26rem)_minmax(0,1fr)]">
        <div className="lg:sticky lg:top-20">
          <BoxHeader box={box} />
          <Card className="mt-5">
            {isSelf ? (
              <div className="flex flex-col gap-3">
                <p className="font-bold">{t("box.own")}</p>
                <Button block icon={<Share2 className="size-4" />} onClick={() => setSharing(true)}>
                  {t("box.ownShare")}
                </Button>
                <Link
                  to="/ask/inbox"
                  className="pressable sticker inline-flex h-11 items-center justify-center gap-2 rounded-full bg-surface px-5 font-bold"
                >
                  <Inbox className="size-4" />
                  {t("hub.inbox")}
                </Link>
              </div>
            ) : box.canAsk ? (
              <AskForm box={box} initialText={draft} />
            ) : (
              <div
                className="flex flex-col items-center gap-3 py-2 text-center"
                data-testid="cannot-ask"
              >
                <div className="text-4xl">{box.open ? "🔐" : "😴"}</div>
                <p className="font-black">{box.open ? t("box.friendsOnly") : t("box.closed")}</p>
                {box.open && (
                  <Link
                    to="/u/$handle"
                    params={{ handle: owner.handle }}
                    className="pressable sticker inline-flex h-10 items-center rounded-full bg-surface px-5 text-sm font-bold"
                  >
                    {t("box.addFriend")}
                  </Link>
                )}
              </div>
            )}
          </Card>

          {openPrompts.length > 0 && (
            <>
              <SectionTitle>{t("box.prompts", { name: owner.displayName })}</SectionTitle>
              <div className="flex flex-col gap-3">
                {openPrompts.map((prompt) => (
                  <PromptCard key={prompt.id} prompt={prompt} />
                ))}
              </div>
            </>
          )}
        </div>

        <section>
          <SectionTitle
            className="lg:mt-0"
            action={
              <span className="text-sm font-bold text-ink-faint">
                {t("box.answersCount", { count: box.answerCount })}
              </span>
            }
          >
            {t("box.answers")}
          </SectionTitle>
          {answers.isPending ? (
            <Skeleton className="h-40" />
          ) : questions.length === 0 ? (
            <EmptyState
              emoji="🫧"
              title={t("box.answersEmpty")}
              body={isSelf ? undefined : t("box.answersEmptyBody")}
            />
          ) : (
            <div className="flex flex-col gap-4">
              {questions.map((question) => (
                <QaCard key={question.id} question={question} />
              ))}
              <LoadMore query={answers} />
            </div>
          )}
        </section>
      </div>
      {sharing && (
        <BoxShareSheet open onOpenChange={setSharing} handle={owner.handle} prompt={box.prompt} />
      )}
    </Page>
  );
}
