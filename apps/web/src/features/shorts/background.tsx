import { useQueryClient } from "@tanstack/react-query";
import { useEffect } from "react";
import { realtime } from "#/lib/realtime";
import forgetShorts from "./forget-user";

/**
 * Runs while someone is signed in. When the other side ends a friendship or
 * puts up a block, the realtime connection says so: the loaded shorts are
 * dropped, as after the viewer's own unfriend or block (friends-only shorts
 * must not keep playing from the cache).
 */
export default function ShortsBackground() {
  const queryClient = useQueryClient();
  useEffect(() => {
    const client = realtime();
    const off = client.onFrame((frame) => {
      if (frame.body.case === "social" && frame.body.value.event.case === "friendRemovedUserId")
        forgetShorts(queryClient);
    });
    client.start();
    return off;
  }, [queryClient]);
  return null;
}
