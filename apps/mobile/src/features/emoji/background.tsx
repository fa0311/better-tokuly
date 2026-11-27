import { useQueryClient } from "@tanstack/react-query";
import { emojiKeys } from "@tokuly/core/emoji";
import { useEffect } from "react";
import { realtime } from "#/lib/realtime";
import { useAfterFirstScreen } from "#/seams/background";
import { syncCatalog } from "./catalog";

/** Everyone may follow this topic; it announces each new version of the catalogue. */
const CATALOG_TOPIC = "emoji:catalog";

/**
 * Runs while someone is signed in: keeps the phone's copy of the emoji
 * catalogue current — once the first screen is up, whenever the server
 * announces a newer version, and again after the connection was down
 * (announcements made meanwhile were missed).
 */
export default function EmojiBackground() {
  const queryClient = useQueryClient();
  const firstScreenUp = useAfterFirstScreen();
  useEffect(() => {
    if (!firstScreenUp) return;
    void syncCatalog();
    const client = realtime();
    const { release } = client.subscribe(CATALOG_TOPIC, { onSubscribed: () => void syncCatalog() });
    const off = client.onFrame((frame) => {
      if (frame.body.case !== "emoji") return;
      const event = frame.body.value.event;
      if (event.case !== "catalogChanged") return;
      void syncCatalog(event.value.version);
      // The deck and an emoji's page may show what changed.
      void queryClient.invalidateQueries({ queryKey: emojiKeys.deck });
      void queryClient.invalidateQueries({ queryKey: emojiKeys.details });
    });
    client.start();
    return () => {
      off();
      release();
    };
  }, [queryClient, firstScreenUp]);
  return null;
}
