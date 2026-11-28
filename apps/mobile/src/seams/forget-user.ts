import type { QueryClient } from "@tanstack/react-query";
import forgetters from "#/generated/seams/forget-user";

/**
 * Forgetting a person: what a feature does when a friendship ends — either
 * side unfriends or blocks, or deletes their account — so that nothing of
 * theirs stays on screen from the cache.
 *
 * A feature that keeps content of other people in its queries or stores
 * default-exports a function from `src/features/<area>/forget-user.ts`:
 *
 *   export default ((queryClient, userId) => {
 *     forgetUserNotes(queryClient, userId);
 *   }) satisfies ForgetUser;
 *
 * The `friendRemovedUserId` push calls `forgetUser(queryClient, userId)`
 * (`features/friends/data.ts`), which runs every one of them: a feature
 * never listens for it itself.
 */
export type ForgetUser = (queryClient: QueryClient, userId: string) => void;

const FORGETTERS: ForgetUser[] = forgetters;

/** Takes everything of `userId` out of what this device has loaded. */
export function forgetUser(queryClient: QueryClient, userId: string): void {
  if (!userId) return;
  for (const forget of FORGETTERS) forget(queryClient, userId);
}
