import type { common, notes } from "@tokuly/api";
import { create } from "zustand";

/** The stories of one person (or of one highlight), in the order they play. */
export interface ViewerReel {
  user: common.UserSummary | undefined;
  stories: notes.Story[];
  /** Set when the reel is a highlight: its title is shown, and its owner can edit it from the viewer. */
  highlight?: notes.Highlight;
}

interface ViewerStore {
  reels: ViewerReel[] | null;
  /** Which reel plays first… */
  reelIndex: number;
  /** …and from which of its stories (by ID; default: the first one not watched yet). */
  storyId?: string;
  /** Called when the viewer closes (a deep link page goes back to where it came from). */
  onClose?: () => void;
}

/** The one story viewer of the app: the tray, a profile ring, a highlight or a link opens it through `openStoryViewer`. */
export const useStoryViewer = create<ViewerStore>(() => ({ reels: null, reelIndex: 0 }));

export function openStoryViewer(reels: ViewerReel[], options: { reelIndex?: number; storyId?: string; onClose?: () => void } = {}): void {
  const playable = reels.filter((reel) => reel.stories.some((story) => story.items.length > 0));
  if (playable.length === 0) return;
  const wanted = reels[options.reelIndex ?? 0];
  useStoryViewer.setState({ reels: playable, reelIndex: Math.max(0, playable.indexOf(wanted!)), storyId: options.storyId, onClose: options.onClose });
}

export function closeStoryViewer(): void {
  const { onClose } = useStoryViewer.getState();
  useStoryViewer.setState({ reels: null, reelIndex: 0, storyId: undefined, onClose: undefined });
  onClose?.();
}
