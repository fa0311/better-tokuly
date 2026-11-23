import type { QueryClient } from "@tanstack/react-query";

/**
 * Forgetting a person: what a feature does when the viewer unfriends or
 * blocks someone, so that nothing of theirs stays on screen from the cache.
 *
 * A feature that keeps content of other people in its queries
 * default-exports a function from `src/features/<area>/forget-user.ts`:
 *
 *   export default ((queryClient, userId) => {
 *     queryClient.removeQueries({ queryKey: notesKeys.userTimelines(userId) });
 *   }) satisfies ForgetUser;
 *
 * The block and unfriend actions call `forgetUser(queryClient, userId)`,
 * which runs every one of them.
 */
export type ForgetUser = (queryClient: QueryClient, userId: string) => void;

const modules = import.meta.glob<{ default: ForgetUser }>("../features/*/forget-user.ts", {
  eager: true,
});

const FORGETTERS = Object.values(modules).map((module) => module.default);

/** Takes everything of `userId` out of what this browser has loaded. */
export function forgetUser(queryClient: QueryClient, userId: string): void {
  if (!userId) return;
  for (const forget of FORGETTERS) forget(queryClient, userId);
}
