import { useQueryClient } from "@tanstack/react-query";
import { Suspense, useEffect, useState } from "react";
import { onDemand } from "#/components/on-demand";
import { onFramesLoading, realtime } from "#/lib/realtime";
import { useSaveTarget } from "./save-target";

const SaveDialogHost = onDemand(
  () => import("./save-dialog").then((module) => module.SaveDialogHost),
  "toast",
);

/**
 * Runs for as long as someone is signed in. A creator's videos change state
 * on their own — processing finishes or fails, a scheduled video goes live —
 * and the server says so over realtime: the cached cards, the studio lists
 * and the open editor follow without a reload. It also hosts the app's one
 * "save to playlist" dialog, which any video card can open.
 */
export default function VideoBackground() {
  const queryClient = useQueryClient();

  useEffect(() => {
    const off = onFramesLoading("video", () =>
      import("./frames").then(
        ({ applyVideoEvent }) =>
          ({ event }) =>
            applyVideoEvent(queryClient, event),
      ),
    );
    realtime().start();
    return off;
  }, [queryClient]);

  // The dialog loads the first time a video is saved, and stays mounted so it can animate out.
  const saving = useSaveTarget((state) => state.video !== null);
  const [opened, setOpened] = useState(false);
  if (saving && !opened) setOpened(true);
  return (
    opened && (
      <Suspense fallback={null}>
        <SaveDialogHost />
      </Suspense>
    )
  );
}
