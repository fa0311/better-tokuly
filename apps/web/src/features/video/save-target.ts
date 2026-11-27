import type { video } from "@tokuly/api";
import { create } from "zustand";

/** The video being saved; the dialog is open while one is set. */
export const useSaveTarget = create<{ video: video.VideoSummary | null }>(() => ({ video: null }));

/** Opens "save to…" for a video, from anywhere in the app. */
export function openSaveDialog(summary: video.VideoSummary): void {
  useSaveTarget.setState({ video: summary });
}
