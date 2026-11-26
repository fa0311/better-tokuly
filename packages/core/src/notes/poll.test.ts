import { create, notes } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import { canVote, newPoll, pollInput, pollReady, pollRemaining, pollShares } from "./poll";

describe("a poll being written", () => {
  it("is ready once every choice is filled in and they all differ", () => {
    const poll = newPoll();
    expect(pollReady(poll)).toBe(false);
    expect(pollReady({ ...poll, choices: ["a", " a "] })).toBe(false);
    expect(pollReady({ ...poll, choices: ["a", "b"] })).toBe(true);
    expect(pollInput({ ...poll, choices: [" a ", "b"] }).choices).toEqual(["a", "b"]);
  });
});

describe("a poll being read", () => {
  it("says how long is left in the largest unit that fits", () => {
    expect(pollRemaining(0, 1)).toEqual(["poll.ended", 0]);
    expect(pollRemaining(90_000, 0)).toEqual(["poll.minutesLeft", 2]);
    expect(pollRemaining(5 * 3_600_000, 0)).toEqual(["poll.hoursLeft", 5]);
    expect(pollRemaining(72 * 3_600_000, 0)).toEqual(["poll.daysLeft", 3]);
  });

  it("shares the votes in whole percent, and takes a vote only once", () => {
    const poll = create(notes.PollSchema, {
      choices: [
        { text: "a", votes: 1n },
        { text: "b", votes: 2n },
      ],
    });
    expect(pollShares(poll)).toEqual([33, 67]);
    expect(canVote(poll, true)).toBe(true);
    expect(canVote({ ...poll, myChoices: [0] }, true)).toBe(false);
    expect(canVote({ ...poll, closed: true }, true)).toBe(false);
    expect(canVote(poll, false)).toBe(false);
  });
});
