import { useQueryClient } from "@tanstack/react-query";
import { shortKeys } from "@tokuly/core/shorts";
import { storyKeys } from "@tokuly/core/stories";
import { useEffect } from "react";
import { AppState } from "react-native";
import { accountsOnDevice } from "#/features/session/account";
import { resumePosts, usePosts } from "./posts";

/**
 * Runs while someone is signed in: posts on their way go on (at start, and
 * whenever the app comes back to the front), and what they became shows up
 * in the lists. They stop before the account leaves (`lifecycle.ts`).
 */
export default function PostsBackground() {
  const queryClient = useQueryClient();

  useEffect(() => {
    const resume = () =>
      void accountsOnDevice()
        .then(resumePosts)
        .catch(() => {});
    resume();
    const appState = AppState.addEventListener("change", (next) => {
      if (next === "active") resume();
    });
    return () => appState.remove();
  }, []);

  useEffect(
    () =>
      usePosts.subscribe((state, previous) => {
        for (const post of state.posts) {
          if (post.status !== "done") continue;
          if (previous.posts.find((held) => held.id === post.id)?.status === "done") continue;
          if (post.kind === "story") {
            void queryClient.invalidateQueries({ queryKey: storyKeys.tray });
            void queryClient.invalidateQueries({ queryKey: ["stories", "user"] });
          } else void queryClient.invalidateQueries({ queryKey: shortKeys.lists });
        }
      }),
    [queryClient],
  );

  return null;
}
