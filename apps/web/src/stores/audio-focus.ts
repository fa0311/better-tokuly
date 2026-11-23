import { create } from "zustand";

/**
 * Who has the microphone and the speakers: a chat call or an audio room (a
 * room or a hangout). Only one at a time, so a private call never goes out
 * to a room's stage and a room never plays over a call.
 *
 * A feature claims focus when it starts using audio and releases it when it
 * stops:
 *
 *   claimAudioFocus({ holder: "call", yield: () => hangUp() });
 *   releaseAudioFocus("call");
 *
 * Claiming while someone else holds focus makes the holder `yield` at once.
 * A holder that can wait (a room pauses rather than ends) also passes
 * `resume`: it is then set aside and gets focus back, through `resume`, when
 * the one that took it releases. A holder without `resume` is simply gone.
 *
 * Taking focus from someone who would lose something (a room asking to end
 * a call) is the claimer's decision to ask the person first: `audioFocusHolder()`
 * says who holds it.
 */
export type AudioHolder = "call" | "room";

export interface AudioClaim {
  holder: AudioHolder;
  /** Stop using the microphone and the speakers now: someone else took them. */
  yield(): void;
  /** Focus is back after the one who took it is done. */
  resume?(): void;
}

interface AudioFocusState {
  current: AudioClaim | null;
  /** A holder that yielded and waits to get focus back. */
  waiting: AudioClaim | null;
}

export const useAudioFocus = create<AudioFocusState>(() => ({ current: null, waiting: null }));

/** Who holds focus right now, if anyone. */
export function audioFocusHolder(): AudioHolder | null {
  return useAudioFocus.getState().current?.holder ?? null;
}

/** Whether `holder` is set aside, waiting for focus to come back. */
export function useAudioFocusWaiting(holder: AudioHolder): boolean {
  return useAudioFocus((state) => state.waiting?.holder === holder);
}

/** Takes focus for `claim.holder`; whoever held it yields first. Claiming again replaces the callbacks. */
export function claimAudioFocus(claim: AudioClaim): void {
  const { current, waiting } = useAudioFocus.getState();
  if (current && current.holder !== claim.holder) {
    useAudioFocus.setState({ current: claim, waiting: current.resume ? current : null });
    current.yield();
    return;
  }
  // The same holder claiming again (a reconnect) keeps whoever is waiting, unless it was itself.
  useAudioFocus.setState({
    current: claim,
    waiting: waiting?.holder === claim.holder ? null : waiting,
  });
}

/** `holder` is done with audio; a holder that was set aside gets focus back. */
export function releaseAudioFocus(holder: AudioHolder): void {
  const { current, waiting } = useAudioFocus.getState();
  if (waiting?.holder === holder) {
    useAudioFocus.setState({ waiting: null });
    return;
  }
  if (current?.holder !== holder) return;
  useAudioFocus.setState({ current: waiting, waiting: null });
  waiting?.resume?.();
}
