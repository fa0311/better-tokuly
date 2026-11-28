import type { QueryClient } from "@tanstack/query-core";
import { reread } from "../query";
import { spacesKeys } from "./keys";

/**
 * A friendship ended: rooms and hangouts shared with friends, and who of the
 * viewer's friends is inside, are read again (a block also takes people out
 * of rooms on the server).
 */
export function forgetUserSpaces(queryClient: QueryClient, _userId: string): Promise<void> {
  return reread(queryClient, [
    spacesKeys.lists,
    spacesKeys.friendsInRooms,
    spacesKeys.hangouts,
    ["spaces", "space"],
    ["spaces", "hangout"],
  ]);
}
