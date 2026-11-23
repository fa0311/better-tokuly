import { useMutation, useQueryClient } from "@tanstack/react-query";
import type { community } from "@tokuly/api";
import { Check } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { EmojiText } from "#/features/emoji";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { boardClient, NS, patchThread } from "./api";
import { votePercent } from "./model";
import { useWhen } from "./parts";

/**
 * A thread's poll: choose (one or several), see the numbers once you voted
 * or it closed, change or take back your vote while it is open.
 */
export function PollCard({
  thread,
  canVote,
}: {
  thread: community.Thread;
  /** The viewer may vote (a member who is not timed out). */
  canVote: boolean;
}) {
  const { t } = useTranslation(NS);
  const when = useWhen();
  const queryClient = useQueryClient();
  const poll = thread.poll;
  const [picked, setPicked] = useState<string[] | null>(null);
  const [changing, setChanging] = useState(false);

  const vote = useMutation({
    mutationFn: (optionIds: string[]) => boardClient().votePoll({ threadId: thread.id, optionIds }),
    onSuccess: ({ poll: next }) => {
      if (next) patchThread(queryClient, thread.id, (each) => ({ ...each, poll: next }));
      setPicked(null);
      setChanging(false);
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: NS })),
  });
  if (!poll) return null;

  const closesAt = poll.closesAt ? Number(poll.closesAt.seconds) * 1000 : 0;
  const voted = poll.myOptionIds.length > 0;
  const choosing = canVote && !poll.closed && (!voted || changing);
  const selection = picked ?? (changing ? poll.myOptionIds : []);
  const toggle = (id: string) =>
    setPicked(
      poll.multipleChoice
        ? selection.includes(id)
          ? selection.filter((each) => each !== id)
          : [...selection, id]
        : [id],
    );

  return (
    <section aria-label={t("poll.title")} className="rounded-2xl border-2 border-line p-3">
      <ul className="flex flex-col gap-2">
        {poll.options.map((option) => {
          const mine = poll.myOptionIds.includes(option.id);
          const percent = poll.resultsVisible ? votePercent(option, poll) : 0;
          if (choosing) {
            const on = selection.includes(option.id);
            return (
              <li key={option.id}>
                <button
                  type="button"
                  aria-pressed={on}
                  onClick={() => toggle(option.id)}
                  className={cn(
                    "flex min-h-11 w-full items-center gap-2.5 rounded-xl border-2 px-3 py-2 text-left font-bold transition-colors",
                    on ? "border-brand bg-brand-soft" : "border-line hover:border-ink-faint",
                  )}
                >
                  <span
                    className={cn(
                      "grid size-5 shrink-0 place-items-center border-2 border-ink",
                      poll.multipleChoice ? "rounded-md" : "rounded-full",
                      on && "bg-brand text-on-brand",
                    )}
                  >
                    {on && <Check className="size-3.5" />}
                  </span>
                  <span className="min-w-0 flex-1 break-words">
                    <EmojiText text={option.text} />
                  </span>
                </button>
              </li>
            );
          }
          return (
            <li
              key={option.id}
              className="relative overflow-hidden rounded-xl border-2 border-line px-3 py-2"
            >
              {poll.resultsVisible && (
                <span
                  aria-hidden
                  className={cn(
                    "absolute inset-y-0 left-0",
                    mine ? "bg-brand-soft" : "bg-surface-2",
                  )}
                  style={{ width: `${percent}%` }}
                />
              )}
              <span className="relative flex items-center gap-2 font-bold">
                {mine && (
                  <Check className="size-4 shrink-0 text-brand" aria-label={t("poll.yours")} />
                )}
                <span className="min-w-0 flex-1 break-words">
                  <EmojiText text={option.text} />
                </span>
                {poll.resultsVisible && (
                  <span className="shrink-0 text-sm text-ink-soft">
                    {t("poll.result", { percent, count: option.voteCount })}
                  </span>
                )}
              </span>
            </li>
          );
        })}
      </ul>
      <div className="mt-3 flex flex-wrap items-center gap-x-3 gap-y-2 text-xs font-bold text-ink-soft">
        <span>{t("poll.voters", { count: poll.voterCount })}</span>
        {poll.multipleChoice && <span>{t("poll.multipleChip")}</span>}
        <span>
          {poll.closed
            ? t("poll.closed")
            : closesAt
              ? t("poll.closesAt", { time: when(closesAt) })
              : t("poll.open")}
        </span>
        <span className="ml-auto flex items-center gap-2">
          {choosing ? (
            <>
              {changing && (
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => {
                    setChanging(false);
                    setPicked(null);
                  }}
                >
                  {t("cancel")}
                </Button>
              )}
              <Button
                size="sm"
                loading={vote.isPending}
                disabled={selection.length === 0}
                onClick={() => vote.mutate(selection)}
              >
                {t("poll.vote")}
              </Button>
            </>
          ) : (
            canVote &&
            !poll.closed &&
            voted && (
              <>
                <Button variant="ghost" size="sm" onClick={() => setChanging(true)}>
                  {t("poll.change")}
                </Button>
                <Button
                  variant="ghost"
                  size="sm"
                  loading={vote.isPending}
                  onClick={() => vote.mutate([])}
                >
                  {t("poll.retract")}
                </Button>
              </>
            )
          )}
        </span>
      </div>
      {!poll.resultsVisible && !choosing && (
        <p className="mt-2 text-xs text-ink-faint">{t("poll.resultsHidden")}</p>
      )}
    </section>
  );
}
