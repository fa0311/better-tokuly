import { describe, expect, it } from "vitest";
import { differsFromShared, expectedPosition, type SharedPlayback, syncAction } from "./sync";

const T0 = 1_800_000_000_000;
const playing: SharedPlayback = { playing: true, positionSeconds: 10, rate: 1, updatedAtMs: T0 };
const paused: SharedPlayback = { playing: false, positionSeconds: 10, rate: 1, updatedAtMs: T0 };

describe("watch party sync", () => {
  it("moves a playing party on with the clock, at its speed, up to the end", () => {
    expect(expectedPosition(playing, T0 + 5_000, 100)).toBe(15);
    expect(expectedPosition({ ...playing, rate: 2 }, T0 + 5_000, 100)).toBe(20);
    expect(expectedPosition(playing, T0 + 500_000, 100)).toBe(100);
    // A clock that runs behind the server's never moves the party backwards.
    expect(expectedPosition(playing, T0 - 3_000, 100)).toBe(10);
  });

  it("keeps a paused party where it was", () => {
    expect(expectedPosition(paused, T0 + 60_000, 100)).toBe(10);
  });

  it("starts and seeks a player that is behind", () => {
    expect(syncAction(playing, { positionSeconds: 0, paused: true, rate: 1 }, T0 + 5_000, 100)).toEqual({ seekTo: 15, play: true });
  });

  it("leaves a player alone that is in step", () => {
    expect(syncAction(playing, { positionSeconds: 15.4, paused: false, rate: 1 }, T0 + 5_000, 100)).toEqual({});
    expect(syncAction(paused, { positionSeconds: 10.1, paused: true, rate: 1 }, T0 + 5_000, 100)).toEqual({});
  });

  it("pulls back a player that drifted", () => {
    expect(syncAction(playing, { positionSeconds: 18, paused: false, rate: 1 }, T0 + 5_000, 100)).toEqual({ seekTo: 15 });
  });

  it("pauses everyone on the host's frame", () => {
    expect(syncAction(paused, { positionSeconds: 12.3, paused: false, rate: 1 }, T0 + 5_000, 100)).toEqual({ pause: true, seekTo: 10 });
  });

  it("follows a change of speed", () => {
    expect(syncAction({ ...playing, rate: 1.5 }, { positionSeconds: 17.5, paused: false, rate: 1 }, T0 + 5_000, 100)).toEqual({ rate: 1.5 });
  });

  it("does not restart a player when the party has played to the end", () => {
    expect(syncAction(playing, { positionSeconds: 100, paused: true, rate: 1 }, T0 + 500_000, 100)).toEqual({});
  });

  it("tells a member's own actions from a player that merely followed", () => {
    // The player did what sync asked: nothing to report.
    expect(differsFromShared(playing, { positionSeconds: 15.2, paused: false, rate: 1 }, T0 + 5_000, 100)).toBe(false);
    expect(differsFromShared(paused, { positionSeconds: 10, paused: true, rate: 1 }, T0 + 5_000, 100)).toBe(false);
    // The member paused, sought or changed the speed.
    expect(differsFromShared(playing, { positionSeconds: 15, paused: true, rate: 1 }, T0 + 5_000, 100)).toBe(true);
    expect(differsFromShared(playing, { positionSeconds: 60, paused: false, rate: 1 }, T0 + 5_000, 100)).toBe(true);
    expect(differsFromShared(playing, { positionSeconds: 15, paused: false, rate: 1.25 }, T0 + 5_000, 100)).toBe(true);
    expect(differsFromShared(paused, { positionSeconds: 10, paused: false, rate: 1 }, T0 + 5_000, 100)).toBe(true);
  });
});
