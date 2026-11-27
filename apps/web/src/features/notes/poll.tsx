import { useQueryClient } from "@tanstack/react-query";
import { type notes, toMillis } from "@tokuly/api";
import { patchNote, pollRemaining } from "@tokuly/core/notes";
import { Check } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { EmojiText } from "#/features/emoji";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { useMe } from "#/stores/session";
import { noteClient } from "./api";
import { useClock } from "./time";

/**
 * A note's poll: the choices to vote for, or the results as bars when the
 * reader may see them. Votes are final and private (nobody sees who voted
 * for what).
 */
export function NotePoll({
  note,
  poll,
  disabled,
}: {
  note: notes.Note;
  poll: notes.Poll;
  disabled?: boolean;
}) {
  const { t } = useTranslation("notes");
  const queryClient = useQueryClient();
  const me = useMe();
  useClock();
  const [picked, setPicked] = useState<number[]>([]);
  const [sending, setSending] = useState(false);
  const voted = poll.myChoices.length > 0;
  const canVote = Boolean(me?.user) && !voted && !poll.closed && !disabled;
  const total = poll.choices.reduce((sum, choice) => sum + Number(choice.votes), 0);

  const send = async (choices: number[]) => {
    setSending(true);
    try {
      const response = await noteClient().votePoll({ noteId: note.id, choices });
      const updated = response.poll;
      if (updated) patchNote(queryClient, note.id, (current) => ({ ...current, poll: updated }));
      toast.success(t("poll.voted"));
    } catch (error) {
      toast.error(errorMessage(error, { namespace: "notes" }));
    } finally {
      setSending(false);
    }
  };

  return (
    <div data-stop-open data-testid="poll" className="flex flex-col gap-1.5">
      {poll.choices.map((choice, index) => {
        const mine = poll.myChoices.includes(index);
        const share = total > 0 ? Math.round((Number(choice.votes) / total) * 100) : 0;
        if (canVote) {
          const selected = picked.includes(index);
          return (
            <button
              key={index}
              type="button"
              disabled={sending}
              aria-pressed={poll.multiple ? selected : undefined}
              onClick={() => {
                if (!poll.multiple) void send([index]);
                else setPicked(selected ? picked.filter((i) => i !== index) : [...picked, index]);
              }}
              className={cn(
                "flex items-center gap-2 rounded-xl border-2 px-3 py-2 text-left text-sm font-bold transition-colors",
                selected ? "border-ink bg-brand-soft" : "border-line hover:border-ink-faint",
              )}
            >
              {poll.multiple && (
                <span
                  className={cn(
                    "grid size-4 shrink-0 place-items-center rounded border-2 border-ink",
                    selected && "bg-lime",
                  )}
                >
                  {selected && <Check className="size-3" strokeWidth={4} />}
                </span>
              )}
              <EmojiText text={choice.text} />
            </button>
          );
        }
        return (
          <div
            key={index}
            className="relative overflow-hidden rounded-xl border-2 border-line px-3 py-2 text-sm font-bold"
          >
            {poll.resultsVisible && (
              <span
                aria-hidden
                className={cn("absolute inset-y-0 left-0", mine ? "bg-lime/60" : "bg-surface-3")}
                style={{ width: `${share}%` }}
              />
            )}
            <span className="relative flex items-center gap-2">
              {mine && (
                <Check className="size-4 shrink-0" strokeWidth={3} aria-label={t("poll.mine")} />
              )}
              <span className="min-w-0 flex-1">
                <EmojiText text={choice.text} />
              </span>
              {poll.resultsVisible && <span className="tabular-nums">{share}%</span>}
            </span>
          </div>
        );
      })}
      {canVote && poll.multiple && (
        <Button
          size="sm"
          className="w-fit"
          disabled={picked.length === 0}
          loading={sending}
          onClick={() => void send([...picked].sort((a, b) => a - b))}
        >
          {t("poll.vote")}
        </Button>
      )}
      <p className="text-xs text-ink-faint">
        {poll.resultsVisible
          ? t("poll.voters", { count: Number(poll.voterCount) })
          : t("poll.hidden")}
        {" · "}
        {(() => {
          const [key, count] = pollRemaining(toMillis(poll.endsAt), Date.now());
          return t(key, { count });
        })()}
      </p>
    </div>
  );
}
