import { media } from "@tokuly/api";
import { create } from "zustand";

const { AssetPurpose } = media;

type Recorded = typeof AssetPurpose.STORY | typeof AssetPurpose.SHORT;

/**
 * The longest recording each purpose takes, in seconds: what the server
 * documents (`Asset.max_duration_seconds`: stories 60, shorts 180) until it
 * says otherwise. Every upload's answer carries the server's own limit, and
 * that is what the camera shows from then on. Longer ones fail with
 * MEDIA_TOO_LONG.
 */
export const useRecordingLimits = create<Record<Recorded, number>>(() => ({
  [AssetPurpose.STORY]: 60,
  [AssetPurpose.SHORT]: 180,
}));

export function recordingLimit(purpose: Recorded): number {
  return useRecordingLimits.getState()[purpose];
}

/** The limit an upload's answer named for its purpose (0: none, or not a recording). */
export function learnLimit(purpose: media.AssetPurpose, seconds: number): void {
  if (seconds <= 0 || (purpose !== AssetPurpose.STORY && purpose !== AssetPurpose.SHORT)) return;
  if (useRecordingLimits.getState()[purpose] !== seconds)
    useRecordingLimits.setState({ [purpose]: seconds });
}
