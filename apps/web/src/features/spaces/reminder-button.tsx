import { useMutation, useQueryClient } from "@tanstack/react-query";
import type { spaces } from "@tokuly/api";
import { patchSpace, rememberSpace, spacesKeys } from "@tokuly/core/spaces";
import { Bell, BellRing } from "lucide-react";
import { toast } from "sonner";
import { Button, type ButtonProps } from "#/components/ui/button";
import { errorMessage } from "#/lib/errors";
import { useTranslation } from "#/lib/use-translation";
import { spaceClient } from "./api";

/** "Tell me when it starts" for an upcoming room; pressed again it takes that back. */
export function ReminderButton({
  space,
  size = "sm",
  block,
}: {
  space: spaces.Space;
  size?: ButtonProps["size"];
  block?: boolean;
}) {
  const { t } = useTranslation("spaces");
  const queryClient = useQueryClient();
  const set = Boolean(space.viewer?.reminderSet);
  const remind = useMutation({
    mutationFn: (next: boolean) => spaceClient().setReminder({ spaceId: space.id, remind: next }),
    onMutate: (next) => {
      // The bell answers the tap at once.
      patchSpace(queryClient, space.id, (loaded) =>
        loaded.viewer
          ? {
              ...loaded,
              reminderCount: Math.max(0, loaded.reminderCount + (next ? 1 : -1)),
              viewer: { ...loaded.viewer, reminderSet: next },
            }
          : loaded,
      );
    },
    onSuccess: (response, next) => {
      rememberSpace(queryClient, response.space);
      void queryClient.invalidateQueries({ queryKey: spacesKeys.lists });
      toast.success(t(next ? "toast.reminderOn" : "toast.reminderOff"));
    },
    onError: (error, next) => {
      patchSpace(queryClient, space.id, (loaded) =>
        loaded.viewer
          ? {
              ...loaded,
              reminderCount: Math.max(0, loaded.reminderCount + (next ? -1 : 1)),
              viewer: { ...loaded.viewer, reminderSet: !next },
            }
          : loaded,
      );
      toast.error(errorMessage(error, { namespace: "spaces" }));
    },
  });
  return (
    <Button
      size={size}
      block={block}
      variant={set ? "lime" : "secondary"}
      aria-pressed={set}
      disabled={remind.isPending}
      icon={set ? <BellRing className="size-4" /> : <Bell className="size-4" />}
      onClick={() => remind.mutate(!set)}
    >
      {t(set ? "reminder.on" : "reminder.off")}
    </Button>
  );
}
