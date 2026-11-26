import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { spaces } from "@tokuly/api";
import { rememberSpace, spacesKeys } from "@tokuly/core/spaces";
import { Pencil, Play, Trash2, X } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button, type ButtonProps } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { errorMessage } from "#/lib/errors";
import { spaceClient } from "./api";
import { adoptSpace, ensureAudioFocus } from "./session";
import { SpaceForm } from "./space-form";

/**
 * What a host can do with a room of their own outside the room itself:
 * start, change or call off an upcoming one, go to a live one, delete one
 * that is over.
 */
export function MySpaceActions({
  space,
  size = "sm",
  onDeleted,
}: {
  space: spaces.Space;
  size?: ButtonProps["size"];
  /** Called once the room is deleted (a page about it has nothing left to show). */
  onDeleted?(): void;
}) {
  const { t } = useTranslation("spaces");
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const [editing, setEditing] = useState(false);
  const [confirming, setConfirming] = useState<"cancel" | "delete" | null>(null);
  const fail = (error: unknown) => toast.error(errorMessage(error, { namespace: "spaces" }));
  const refreshLists = () => void queryClient.invalidateQueries({ queryKey: spacesKeys.lists });

  const start = useMutation({
    mutationFn: async () =>
      (await ensureAudioFocus()) ? spaceClient().startSpace({ spaceId: space.id }) : null,
    onSuccess: async (response) => {
      if (!response) return;
      refreshLists();
      if (!response.space) return;
      void navigate({ to: "/spaces/$spaceId", params: { spaceId: space.id } });
      await adoptSpace(queryClient, response.space, response.credentials);
    },
    onError: fail,
  });
  const cancel = useMutation({
    mutationFn: () => spaceClient().cancelSpace({ spaceId: space.id }),
    onSuccess: (response) => {
      rememberSpace(queryClient, response.space);
      refreshLists();
      setConfirming(null);
      toast.success(t("toast.cancelled"));
    },
    onError: fail,
  });
  const remove = useMutation({
    mutationFn: () => spaceClient().deleteSpace({ spaceId: space.id }),
    onSuccess: () => {
      queryClient.setQueryData(spacesKeys.space(space.id), null);
      refreshLists();
      setConfirming(null);
      toast.success(t("toast.deleted"));
      onDeleted?.();
    },
    onError: fail,
  });

  const state = space.state;
  return (
    <>
      {state === spaces.SpaceState.SCHEDULED && (
        <>
          <Button
            size={size}
            variant="lime"
            icon={<Play className="size-4" />}
            loading={start.isPending}
            onClick={() => start.mutate()}
          >
            {t("mine.start")}
          </Button>
          <Button
            size={size}
            variant="secondary"
            icon={<Pencil className="size-4" />}
            onClick={() => setEditing(true)}
          >
            {t("mine.edit")}
          </Button>
          <Button
            size={size}
            variant="ghost"
            icon={<X className="size-4" />}
            onClick={() => setConfirming("cancel")}
          >
            {t("mine.cancel")}
          </Button>
        </>
      )}
      {state === spaces.SpaceState.LIVE && (
        <Link
          to="/spaces/$spaceId"
          params={{ spaceId: space.id }}
          className="pressable sticker inline-flex h-9 items-center rounded-full bg-brand px-4 text-sm font-bold text-on-brand"
        >
          {t("mine.open")}
        </Link>
      )}
      {(state === spaces.SpaceState.ENDED || state === spaces.SpaceState.CANCELLED) && (
        <Button
          size={size}
          variant="ghost"
          icon={<Trash2 className="size-4" />}
          onClick={() => setConfirming("delete")}
        >
          {t("mine.delete")}
        </Button>
      )}
      <SpaceForm open={editing} onOpenChange={setEditing} space={space} />
      <ConfirmDialog
        open={confirming === "cancel"}
        onOpenChange={(open) => !open && setConfirming(null)}
        title={t("confirm.cancelTitle")}
        description={t("confirm.cancelBody")}
        confirmLabel={t("mine.cancel")}
        danger
        loading={cancel.isPending}
        onConfirm={() => cancel.mutate()}
      />
      <ConfirmDialog
        open={confirming === "delete"}
        onOpenChange={(open) => !open && setConfirming(null)}
        title={t("confirm.deleteTitle")}
        description={t("confirm.deleteBody")}
        confirmLabel={t("mine.delete")}
        danger
        loading={remove.isPending}
        onConfirm={() => remove.mutate()}
      />
    </>
  );
}
