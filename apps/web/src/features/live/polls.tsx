import { useMutation, useQueryClient } from "@tanstack/react-query";
import { type live, toMillis } from "@tokuly/api";
import { BarChart3, Check, Plus, X } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Dialog } from "#/components/ui/dialog";
import { TextField } from "#/components/ui/field";
import { EmojiText } from "#/features/emoji";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { liveKeys, patchStream, pollClient } from "./api";
import { mergePoll, pollShares } from "./model";
import { formatCount, useNow } from "./parts";
import { clock } from "./player/live-player";

/** How long a closed poll keeps showing its result to viewers. */
export const POLL_RESULT_SECONDS = 20;

/** Whether a poll still belongs on screen: open, or closed a moment ago. */
export function pollVisible(poll: live.Poll | undefined, nowMs: number): poll is live.Poll {
  if (!poll) return false;
  if (!poll.endedAt) return true;
  return nowMs - toMillis(poll.endedAt) < POLL_RESULT_SECONDS * 1000;
}

/**
 * A poll in a stream: tap an option to cast the one vote, then watch the
 * bars move as the votes of everyone come in. The streamer and moderators
 * can close it early.
 */
export function PollCard({
  poll,
  canVote,
  canManage,
  className,
}: {
  poll: live.Poll;
  canVote: boolean;
  canManage: boolean;
  className?: string;
}) {
  const { t, i18n } = useTranslation("live");
  const queryClient = useQueryClient();
  const now = useNow();
  const closed = poll.endedAt !== undefined;
  const left = poll.endsAt ? Math.max(0, Math.ceil((toMillis(poll.endsAt) - now) / 1000)) : 0;
  const shares = pollShares(poll);
  const voted = poll.viewerChoice > 0;
  const showResults = voted || closed || !canVote;
  const leading = Math.max(...poll.options.map((option) => Number(option.voteCount)));

  const store = (next: live.Poll | undefined) => {
    if (!next) return;
    patchStream(queryClient, poll.streamId, (stream) => ({
      ...stream,
      activePoll: mergePoll(stream.activePoll, next),
    }));
    void queryClient.invalidateQueries({ queryKey: liveKeys.polls(poll.streamId) });
  };
  const vote = useMutation({
    mutationFn: (option: number) => pollClient().votePoll({ pollId: poll.id, option }),
    onSuccess: (response) => store(response.poll),
    onError: (error) => toast.error(errorMessage(error, { namespace: "live" })),
  });
  const end = useMutation({
    mutationFn: () => pollClient().endPoll({ pollId: poll.id }),
    onSuccess: (response) => store(response.poll),
    onError: (error) => toast.error(errorMessage(error, { namespace: "live" })),
  });

  return (
    <section
      className={cn("sticker-sm rounded-2xl bg-surface p-3", className)}
      aria-label={t("poll.title")}
      data-testid="poll-card"
    >
      <div className="flex items-start gap-2">
        <BarChart3 className="mt-0.5 size-4 shrink-0 text-brand" />
        <h3 className="min-w-0 flex-1 text-sm leading-snug font-black">
          <EmojiText text={poll.question} />
        </h3>
        <span className="shrink-0 text-xs font-bold text-ink-soft tabular-nums">
          {closed ? t("poll.closed") : t("poll.left", { clock: clock(left) })}
        </span>
      </div>
      <ul className="mt-2 flex flex-col gap-1.5">
        {poll.options.map((option, index) => {
          const choice = index + 1;
          const mine = poll.viewerChoice === choice;
          const wins = closed && leading > 0 && Number(option.voteCount) === leading;
          const body = (
            <>
              <span
                className={cn(
                  "absolute inset-y-0 left-0 rounded-xl transition-[width] duration-500 ease-out",
                  mine || wins ? "bg-lime" : "bg-brand-soft",
                )}
                style={{ width: showResults ? `${shares[index]}%` : "0%" }}
                aria-hidden
              />
              <span className="relative flex min-w-0 flex-1 items-center gap-1.5">
                {mine && <Check className="size-4 shrink-0" aria-label={t("poll.yourVote")} />}
                <span className="truncate">
                  <EmojiText text={option.text} />
                </span>
              </span>
              {showResults && (
                <span className="relative shrink-0 tabular-nums" data-testid="poll-share">
                  {shares[index]}%
                </span>
              )}
            </>
          );
          const classes =
            "relative flex h-10 w-full items-center gap-2 overflow-hidden rounded-xl border-2 border-line px-3 text-left text-sm font-bold";
          return (
            <li key={index}>
              {canVote && !voted && !closed ? (
                <button
                  type="button"
                  disabled={vote.isPending}
                  onClick={() => vote.mutate(choice)}
                  className={cn(classes, "hover:border-brand")}
                >
                  {body}
                </button>
              ) : (
                <div className={classes}>{body}</div>
              )}
            </li>
          );
        })}
      </ul>
      <div className="mt-2 flex items-center justify-between gap-2">
        <span className="text-xs font-bold text-ink-faint" data-testid="poll-votes">
          {t("poll.votes", { formatted: formatCount(poll.totalVotes, i18n.language) })}
        </span>
        {canManage && !closed && (
          <Button variant="ghost" size="sm" loading={end.isPending} onClick={() => end.mutate()}>
            {t("poll.end")}
          </Button>
        )}
      </div>
    </section>
  );
}

const DURATIONS = [30, 60, 120, 300, 600] as const;

/** Opens a poll: a question, two to five options, how long it runs. */
export function CreatePollDialog({
  streamId,
  open,
  onOpenChange,
}: {
  streamId: string;
  open: boolean;
  onOpenChange(open: boolean): void;
}) {
  const { t } = useTranslation("live");
  const queryClient = useQueryClient();
  const [question, setQuestion] = useState("");
  const [options, setOptions] = useState(["", ""]);
  const [duration, setDuration] = useState<number>(60);
  const filled = options.map((option) => option.trim()).filter(Boolean);
  const create = useMutation({
    mutationFn: () =>
      pollClient().createPoll({
        streamId,
        question: question.trim(),
        options: filled,
        durationSeconds: duration,
      }),
    onSuccess: ({ poll }) => {
      if (poll) patchStream(queryClient, streamId, (stream) => ({ ...stream, activePoll: poll }));
      void queryClient.invalidateQueries({ queryKey: liveKeys.polls(streamId) });
      toast.success(t("poll.created"));
      setQuestion("");
      setOptions(["", ""]);
      onOpenChange(false);
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "live" })),
  });
  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("poll.createTitle")}
      description={t("poll.createLead")}
      footer={
        <Button
          loading={create.isPending}
          disabled={!question.trim() || filled.length < 2}
          onClick={() => create.mutate()}
        >
          {t("poll.start")}
        </Button>
      }
    >
      <div className="flex flex-col gap-3">
        <TextField
          label={t("poll.question")}
          value={question}
          maxLength={100}
          onChange={(event) => setQuestion(event.target.value)}
        />
        {options.map((option, index) => (
          <TextField
            key={index}
            label={t("poll.option", { number: index + 1 })}
            value={option}
            maxLength={50}
            onChange={(event) =>
              setOptions(options.map((value, at) => (at === index ? event.target.value : value)))
            }
            trailing={
              options.length > 2 && (
                <button
                  type="button"
                  aria-label={t("poll.removeOption", { number: index + 1 })}
                  onClick={() => setOptions(options.filter((_, at) => at !== index))}
                  className="grid size-9 place-items-center rounded-full text-ink-soft hover:bg-surface-2"
                >
                  <X className="size-4" />
                </button>
              )
            }
          />
        ))}
        {options.length < 5 && (
          <Button
            variant="ghost"
            size="sm"
            className="self-start"
            icon={<Plus className="size-4" />}
            onClick={() => setOptions([...options, ""])}
          >
            {t("poll.addOption")}
          </Button>
        )}
        <fieldset className="flex flex-wrap gap-2">
          <legend className="mb-2 px-1 text-sm font-bold text-ink-soft">
            {t("poll.duration")}
          </legend>
          {DURATIONS.map((seconds) => (
            <button
              key={seconds}
              type="button"
              aria-pressed={duration === seconds}
              onClick={() => setDuration(seconds)}
              className={cn(
                "h-9 rounded-full border-2 px-3.5 text-sm font-black",
                duration === seconds
                  ? "border-ink bg-sun text-[#17122b]"
                  : "border-line text-ink-soft hover:text-ink",
              )}
            >
              {seconds < 60
                ? t("poll.seconds", { count: seconds })
                : t("poll.minutes", { count: seconds / 60 })}
            </button>
          ))}
        </fieldset>
      </div>
    </Dialog>
  );
}
