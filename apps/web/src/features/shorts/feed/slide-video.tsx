import type { shorts } from "@tokuly/api";
import { type Ref, type RefObject, useEffect, useImperativeHandle, useRef } from "react";
import { reportStay, reportStayOnLeave } from "../stay";
import { type ShortVideoState, useShortVideo } from "./use-short-video";

export interface SlideVideoHandle {
  toggle(): void;
  setFast(fast: boolean): void;
}

/**
 * The `<video>` of a slide. Mounted for the short on screen and the one
 * after it only; unmounting gives the stream back. While `active` it plays
 * and loops, and each time the viewer leaves, the stay is reported.
 */
export function SlideVideo({
  short,
  active,
  source,
  progress,
  onState,
  ref,
}: {
  short: shorts.Short;
  active: boolean;
  source: shorts.PlaybackSource;
  /** The progress bar's fill: scaled as playback moves. */
  progress: RefObject<HTMLElement | null>;
  onState(state: ShortVideoState): void;
  ref: Ref<SlideVideoHandle>;
}) {
  const video = useRef<HTMLVideoElement>(null);
  const [state, controls] = useShortVideo(video, short.playback, active, progress);
  useImperativeHandle(ref, () => ({ toggle: controls.toggle, setFast: controls.setFast }));

  const { paused, waiting, failed } = state;
  useEffect(() => onState({ paused, waiting, failed }), [onState, paused, waiting, failed]);

  // One report per stay: when the viewer swipes on, leaves the feed or closes the page.
  // The owner's own plays count for nothing, so they are not sent.
  const { takeStay } = controls;
  const shortId = short.id;
  const own = short.viewer?.isOwner ?? false;
  useEffect(() => {
    if (!active || own) return;
    const onLeave = () => reportStayOnLeave(shortId, source, takeStay());
    window.addEventListener("pagehide", onLeave);
    return () => {
      window.removeEventListener("pagehide", onLeave);
      reportStay(shortId, source, takeStay());
    };
  }, [active, own, shortId, source, takeStay]);

  return (
    <video
      ref={video}
      data-testid="short-video"
      className="absolute inset-0 size-full object-cover"
      playsInline
      preload="auto"
      disablePictureInPicture
      // The cover underneath shows until the first frame is there.
      aria-hidden
      tabIndex={-1}
    />
  );
}
