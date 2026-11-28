import { useMutation, useQueryClient } from "@tanstack/react-query";
import type { spaces } from "@tokuly/api";
import { rememberSpace, spacesKeys } from "@tokuly/core/spaces";
import { UserPlus } from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Dialog } from "#/components/ui/dialog";
import { TextArea, TextField } from "#/components/ui/field";
import { LoadMore } from "#/components/ui/load-more";
import { PaneSpinner } from "#/components/ui/spinner";
import { errorMessage } from "#/lib/errors";
import { useTranslation } from "#/lib/use-translation";
import { spaceClient, useBans, useSpaceInvites } from "../api";
import { FriendPicker } from "../friend-picker";
import { PersonRow } from "./people";

const fail = (error: unknown) => toast.error(errorMessage(error, { namespace: "spaces" }));

/** Pins a note, a link or both at the top of the room; both empty unpins. */
export function PinDialog({
  space,
  open,
  onOpenChange,
}: {
  space: spaces.Space;
  open: boolean;
  onOpenChange(open: boolean): void;
}) {
  const { t } = useTranslation("spaces");
  const queryClient = useQueryClient();
  const [text, setText] = useState(space.pin?.text ?? "");
  const [url, setUrl] = useState(space.pin?.url ?? "");
  const [wasOpen, setWasOpen] = useState(open);
  if (open !== wasOpen) {
    setWasOpen(open);
    if (open) {
      setText(space.pin?.text ?? "");
      setUrl(space.pin?.url ?? "");
    }
  }
  const save = useMutation({
    mutationFn: (pin: { text: string; url: string }) =>
      spaceClient().setPin({ spaceId: space.id, ...pin }),
    onSuccess: (response, pin) => {
      rememberSpace(queryClient, response.space);
      onOpenChange(false);
      toast.success(t(pin.text || pin.url ? "toast.pinned" : "toast.unpinned"));
    },
    onError: fail,
  });
  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("pin.title")}
      description={t("pin.body")}
      footer={
        <>
          {space.pin && (
            <Button
              variant="ghost"
              disabled={save.isPending}
              onClick={() => save.mutate({ text: "", url: "" })}
            >
              {t("pin.unpin")}
            </Button>
          )}
          <Button
            loading={save.isPending}
            disabled={!text.trim() && !url.trim()}
            onClick={() => save.mutate({ text: text.trim(), url: url.trim() })}
          >
            {t("pin.save")}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-4">
        <TextArea
          label={t("pin.text")}
          placeholder={t("pin.textPlaceholder")}
          value={text}
          maxLength={200}
          onChange={(event) => setText(event.target.value)}
        />
        <TextField
          label={t("pin.url")}
          type="url"
          inputMode="url"
          placeholder="https://"
          value={url}
          maxLength={2000}
          onChange={(event) => setUrl(event.target.value)}
        />
      </div>
    </Dialog>
  );
}

/** Invites friends to the room; moderators also see who holds an invitation and can take it back. */
export function InvitesDialog({
  space,
  moderator,
  open,
  onOpenChange,
}: {
  space: spaces.Space;
  moderator: boolean;
  open: boolean;
  onOpenChange(open: boolean): void;
}) {
  const { t } = useTranslation("spaces");
  const queryClient = useQueryClient();
  const [picking, setPicking] = useState(false);
  const invites = useSpaceInvites(space.id, open && moderator);
  const invited = invites.data?.pages.flatMap((page) => page.invites) ?? [];
  const refresh = () =>
    void queryClient.invalidateQueries({ queryKey: spacesKeys.invites(space.id) });

  const invite = useMutation({
    mutationFn: (userIds: string[]) => spaceClient().inviteToSpace({ spaceId: space.id, userIds }),
    onSuccess: (response) => {
      setPicking(false);
      refresh();
      toast.success(
        response.invitedCount > 0
          ? t("toast.invited", { count: response.invitedCount })
          : t("toast.invitedNone"),
      );
      if (!moderator) onOpenChange(false);
    },
    onError: fail,
  });
  const revoke = useMutation({
    mutationFn: (userId: string) => spaceClient().revokeSpaceInvite({ spaceId: space.id, userId }),
    onSuccess: refresh,
    onError: fail,
  });

  const picker = (
    <FriendPicker
      open={moderator ? picking : open}
      onOpenChange={moderator ? setPicking : onOpenChange}
      title={t("invites.pick")}
      description={t("invites.pickBody")}
      exclude={invited.flatMap((entry) => (entry.user ? [entry.user.id] : []))}
      confirmLabel={t("invites.send")}
      loading={invite.isPending}
      onConfirm={(users) => {
        if (users.length === 0) return moderator ? setPicking(false) : onOpenChange(false);
        invite.mutate(users.map((user) => user.id));
      }}
    />
  );
  // Whoever is not a moderator goes straight to choosing friends.
  if (!moderator) return picker;

  return (
    <>
      <Dialog
        open={open && !picking}
        onOpenChange={onOpenChange}
        title={t("invites.title")}
        footer={
          <Button block icon={<UserPlus className="size-4" />} onClick={() => setPicking(true)}>
            {t("invites.add")}
          </Button>
        }
      >
        {invites.isPending ? (
          <PaneSpinner />
        ) : invited.length === 0 ? (
          <p className="py-6 text-center text-sm font-bold text-ink-faint">{t("invites.empty")}</p>
        ) : (
          <ul className="flex flex-col">
            {invited.map((entry) => (
              <PersonRow
                key={entry.user?.id}
                user={entry.user}
                trailing={
                  <Button
                    size="sm"
                    variant="ghost"
                    disabled={revoke.isPending}
                    onClick={() => entry.user && revoke.mutate(entry.user.id)}
                  >
                    {t("invites.revoke")}
                  </Button>
                }
              />
            ))}
            <LoadMore query={invites} />
          </ul>
        )}
      </Dialog>
      {picker}
    </>
  );
}

/** The people banned from the room, with a way to let them back in. */
export function BansDialog({
  space,
  open,
  onOpenChange,
}: {
  space: spaces.Space;
  open: boolean;
  onOpenChange(open: boolean): void;
}) {
  const { t } = useTranslation("spaces");
  const queryClient = useQueryClient();
  const bans = useBans(space.id, open);
  const banned = bans.data?.pages.flatMap((page) => page.bans) ?? [];
  const unban = useMutation({
    mutationFn: (userId: string) => spaceClient().unbanUser({ spaceId: space.id, userId }),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: spacesKeys.bans(space.id) });
      toast.success(t("toast.unbanned"));
    },
    onError: fail,
  });
  return (
    <Dialog open={open} onOpenChange={onOpenChange} title={t("bans.title")}>
      {bans.isPending ? (
        <PaneSpinner />
      ) : banned.length === 0 ? (
        <p className="py-6 text-center text-sm font-bold text-ink-faint">{t("bans.empty")}</p>
      ) : (
        <ul className="flex flex-col">
          {banned.map((ban) => (
            <PersonRow
              key={ban.user?.id}
              user={ban.user}
              trailing={
                <Button
                  size="sm"
                  variant="secondary"
                  disabled={unban.isPending}
                  onClick={() => ban.user && unban.mutate(ban.user.id)}
                >
                  {t("bans.unban")}
                </Button>
              }
            />
          ))}
          <LoadMore query={bans} />
        </ul>
      )}
    </Dialog>
  );
}
