import { notes } from "@tokuly/api";

/** A poll as a composer holds it. */
export interface PollDraft {
  choices: string[];
  multiple: boolean;
  durationSeconds: number;
  results: notes.PollResultsVisibility;
}

export const POLL_MAX_CHOICES = 10;
export const POLL_CHOICE_MAX_CHARS = 50;
/** The lengths a composer offers: 5 minutes to a week (the server takes up to 30 days). */
export const POLL_DURATIONS = [300, 3600, 6 * 3600, 86_400, 3 * 86_400, 7 * 86_400] as const;

export function newPoll(): PollDraft {
  return {
    choices: ["", ""],
    multiple: false,
    durationSeconds: 86_400,
    results: notes.PollResultsVisibility.ALWAYS,
  };
}

/** What CreateNote takes for the poll. */
export function pollInput(poll: PollDraft) {
  return {
    choices: poll.choices.map((choice) => choice.trim()),
    multiple: poll.multiple,
    durationSeconds: BigInt(poll.durationSeconds),
    resultsVisibility: poll.results,
  };
}

/** Whether every choice is filled in and they all differ. */
export function pollReady(poll: PollDraft): boolean {
  const choices = poll.choices.map((choice) => choice.trim());
  return choices.every(Boolean) && new Set(choices).size === choices.length;
}

/** How long until a poll's end, as a `notes:poll.*` string key and its count. */
export function pollRemaining(endsMs: number, nowMs: number): [string, number] {
  const left = endsMs - nowMs;
  if (left <= 0) return ["poll.ended", 0];
  const minutes = Math.ceil(left / 60_000);
  if (minutes < 60) return ["poll.minutesLeft", minutes];
  const hours = Math.ceil(minutes / 60);
  if (hours < 48) return ["poll.hoursLeft", hours];
  return ["poll.daysLeft", Math.ceil(hours / 24)];
}

/** Each choice's share of the votes, in whole percent. */
export function pollShares(poll: notes.Poll): number[] {
  const total = poll.choices.reduce((sum, choice) => sum + Number(choice.votes), 0);
  return poll.choices.map((choice) =>
    total > 0 ? Math.round((Number(choice.votes) / total) * 100) : 0,
  );
}

/** Whether the reader may still vote: signed in, not voted yet, the poll open. */
export function canVote(poll: notes.Poll, signedIn: boolean): boolean {
  return signedIn && poll.myChoices.length === 0 && !poll.closed;
}
