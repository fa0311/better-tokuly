import { useMutation, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { errorMessage } from "#/lib/errors";
import { keys } from "../api";

/** Something a person does in a room: the call, and what to say when it worked. */
export interface RoomAction {
  call(): Promise<unknown>;
  /** A toast for success; most actions show for themselves on the stage. */
  done?: string;
  /** Runs after success, before the lists of people are asked for again. */
  after?(): void;
}

/**
 * Runs the actions of a room (raise a hand, invite to the stage, mute,
 * remove…) one at a time: failures are said in the API's words, and the
 * lists of people are refreshed afterwards, since not every change is
 * announced to the person who made it.
 */
export function useRoomAction(spaceId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (action: RoomAction) => action.call(),
    onSuccess: (_result, action) => {
      if (action.done) toast.success(action.done);
      action.after?.();
      void queryClient.invalidateQueries({ queryKey: keys.everyParticipants(spaceId) });
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "spaces" })),
  });
}
