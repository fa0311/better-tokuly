import type { notes } from "@tokuly/api";
import { create } from "zustand";

/** What a new note is written in answer to. */
export interface ComposeContext {
  /** The note being answered. */
  replyTo?: notes.Note;
  /** The note being quoted. */
  quote?: notes.Note;
  /** Text to start from when there is no draft (a hashtag page offers its `#tag`). */
  text?: string;
}

interface ComposeStore {
  open: boolean;
  context: ComposeContext;
}

/** The one composer of the app: any button, shortcut or card opens it through `openCompose`. */
export const useCompose = create<ComposeStore>(() => ({ open: false, context: {} }));

/** Opens the composer for a new note, a reply or a quote. */
export function openCompose(context: ComposeContext = {}): void {
  useCompose.setState({ open: true, context });
}

export function closeCompose(): void {
  useCompose.setState({ open: false });
}
