import { shouldScrollToNewOwnEntry } from "../timeline-scroll";

describe("shouldScrollToNewOwnEntry", () => {
  it("scrolls when a newly posted note becomes the newest entry", () => {
    expect(shouldScrollToNewOwnEntry("friend-note", "own-note", "me", "me")).toBe(true);
  });

  it("does not scroll for a friend update", () => {
    expect(shouldScrollToNewOwnEntry("friend-note", "new-friend-note", "friend", "me")).toBe(false);
  });

  it("does not scroll for the initial timeline load", () => {
    expect(shouldScrollToNewOwnEntry(undefined, "own-note", "me", "me")).toBe(false);
  });

  it("does not scroll when the current top row is unchanged", () => {
    expect(shouldScrollToNewOwnEntry("own-note", "own-note", "me", "me")).toBe(false);
  });
});
