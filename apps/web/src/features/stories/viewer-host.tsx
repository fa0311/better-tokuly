import { lazy, Suspense } from "react";
import { useStoryViewer } from "./viewer-store";

const StoryViewer = lazy(() =>
  import("./viewer").then((module) => ({ default: module.StoryViewer })),
);

/** The viewer, mounted once with the stories background; it loads the first time a story opens. */
export function StoryViewerHost() {
  const reels = useStoryViewer((state) => state.reels);
  const reelIndex = useStoryViewer((state) => state.reelIndex);
  const storyId = useStoryViewer((state) => state.storyId);
  if (!reels) return null;
  return (
    <Suspense fallback={null}>
      <StoryViewer reels={reels} startReel={reelIndex} startStoryId={storyId} />
    </Suspense>
  );
}
