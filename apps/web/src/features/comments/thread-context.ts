import type { common } from "@tokuly/api";
import { createContext, useContext } from "react";
import type { SubjectKey } from "./api";

/** The player of the video a thread belongs to, so comments can point at moments of it. */
export interface CommentVideoBridge {
  /** Where playback is right now, in seconds. */
  currentTime(): number;
  seekTo(seconds: number): void;
}

/** What every part of one thread needs to know about it. */
export interface ThreadContextValue {
  subject: SubjectKey;
  /** The viewer owns the subject: pin, heart, hide, block. */
  canModerate: boolean;
  /** The subject's own public author, for the creator badge. */
  subjectAuthor?: common.Author;
  video?: CommentVideoBridge;
  /** The channel new comments are written as unless the person picks another identity. */
  defaultChannelId?: string;
}

export const ThreadContext = createContext<ThreadContextValue | null>(null);

export function useThread(): ThreadContextValue {
  const value = useContext(ThreadContext);
  if (!value) throw new Error("comment parts must be rendered inside <CommentThread>");
  return value;
}
