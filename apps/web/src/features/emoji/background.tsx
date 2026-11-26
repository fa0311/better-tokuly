import { useQueryClient } from "@tanstack/react-query";
import { emojiKeys } from "@tokuly/core/emoji";
import { useEffect } from "react";
import { realtime } from "#/lib/realtime";
import { syncCatalog } from "./catalog";
import { bindUseReporting } from "./use";

/** Everyone may follow this topic; it announces each new version of the catalogue. */
const CATALOG_TOPIC = "emoji:catalog";

/**
 * Runs while someone is signed in: keeps this browser's copy of the emoji
 * catalogue current (once at start, then whenever the server announces a
 * newer version, and again after the connection was down), and refreshes the
 * person's own submissions, imports and exports when the server pushes news
 * about them.
 */
export default function EmojiBackground() {
  const queryClient = useQueryClient();

  useEffect(() => {
    bindUseReporting(queryClient);
    void syncCatalog();
    const client = realtime();
    // In effect (first, or back after a gap): announcements made before were missed, including
    // while the catalogue was first being read, so ask for changes (cheap when there are none).
    const { release } = client.subscribe(CATALOG_TOPIC, { onSubscribed: () => void syncCatalog() });
    const off = client.onFrame((frame) => {
      if (frame.body.case !== "emoji") return;
      const event = frame.body.value.event;
      if (event.case === "catalogChanged") {
        void syncCatalog(event.value.version);
        // Lists drawn from the server (popular, newest, search, an emoji's page) may show what changed.
        for (const key of [
          emojiKeys.popular,
          emojiKeys.newest,
          ["emoji", "search"],
          ["emoji", "author"],
          emojiKeys.details,
          emojiKeys.deck,
        ])
          void queryClient.invalidateQueries({ queryKey: key });
      } else if (event.case === "submissionUpdated") {
        void queryClient.invalidateQueries({ queryKey: emojiKeys.submissions });
        void queryClient.invalidateQueries({ queryKey: emojiKeys.submitter });
        void queryClient.invalidateQueries({ queryKey: emojiKeys.details });
      } else if (event.case === "importUpdated") {
        void queryClient.invalidateQueries({
          queryKey: emojiKeys.adminImport(event.value.id),
          exact: true,
        });
        void queryClient.invalidateQueries({ queryKey: emojiKeys.adminImports });
      } else if (event.case === "exportUpdated") {
        void queryClient.invalidateQueries({ queryKey: emojiKeys.adminExports });
      }
    });
    client.start();
    return () => {
      off();
      release();
    };
  }, [queryClient]);

  return null;
}
