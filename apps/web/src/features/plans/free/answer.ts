import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type { plans } from "@tokuly/api";
import { toast } from "sonner";
import { errorMessage } from "#/lib/errors";
import { type FreeBoard, freeClient, keys } from "../api";

/**
 * "あそぼ!" to a friend's status. The face in the strip shows it as sent at
 * once; the answer comes back with a plan for the two, ready to be edited.
 */
export function useAnswerFree() {
  const queryClient = useQueryClient();
  const mark = (userId: string, answered: boolean) =>
    queryClient.setQueryData<FreeBoard>(keys.freeFriends, (board) =>
      board ? { ...board, statuses: board.statuses.map((status) => (status.user?.id === userId ? { ...status, answeredByMe: answered } : status)) } : board,
    );
  return useMutation({
    mutationFn: ({ status, message }: { status: plans.FreeStatus; message?: string }) => freeClient().answerFreeStatus({ userId: status.user?.id ?? "", message: message?.trim() ?? "" }),
    onMutate: ({ status }) => {
      const before = status.answeredByMe;
      if (status.user) mark(status.user.id, true);
      return { before };
    },
    onError: (error, { status }, context) => {
      if (status.user) mark(status.user.id, context?.before ?? false);
      toast.error(errorMessage(error, { namespace: "plans" }));
      // A status that is gone should leave the strip.
      void queryClient.invalidateQueries({ queryKey: keys.freeFriends });
    },
  });
}

/** The friends the person asked to be told about, and switching one on or off. */
export function useFreeAlerts() {
  const queryClient = useQueryClient();
  const query = useQuery({
    queryKey: keys.freeAlerts,
    queryFn: async () => (await freeClient().listFreeAlerts({ page: { limit: 200, cursor: "" } })).users,
  });
  const set = useMutation({
    mutationFn: ({ userIds, enabled }: { userIds: string[]; enabled: boolean }) => Promise.all(userIds.map((userId) => freeClient().setFreeAlert({ userId, enabled }))),
    onSettled: () => queryClient.invalidateQueries({ queryKey: keys.freeAlerts }),
    onError: (error) => toast.error(errorMessage(error, { namespace: "plans" })),
  });
  return { users: query.data ?? [], isPending: query.isPending, set };
}
