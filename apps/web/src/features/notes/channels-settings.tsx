import { useMutation, useQueryClient } from "@tanstack/react-query";
import { errorReason, type notes } from "@tokuly/api";
import { UserPlus } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { Dialog } from "#/components/ui/dialog";
import { TextField } from "#/components/ui/field";
import { Skeleton } from "#/components/ui/page";
import { Tabs } from "#/components/ui/tabs";
import { profileClient } from "#/lib/api";
import { errorMessage } from "#/lib/errors";
import { useMe } from "#/stores/session";
import { channelClient, channelKeys, storeChannel, useChannelModerators } from "./channels-api";
import { ChannelForm } from "./channels-form";

type ManageTab = "edit" | "moderators" | "archive";

/**
 * Settings of a channel for its owner and moderators: editing it (not
 * while archived), its moderators (the owner names and removes them; a
 * moderator may step down) and, for the owner, archiving it.
 */
export function ChannelManageDialog({
  channel,
  onClose,
}: {
  channel: notes.NoteChannel;
  onClose(): void;
}) {
  const { t } = useTranslation("channels");
  const tabs: ManageTab[] = [
    ...(channel.archived ? [] : (["edit"] as const)),
    "moderators",
    ...(channel.isOwner ? (["archive"] as const) : []),
  ];
  const [tab, setTab] = useState<ManageTab>(tabs[0]!);
  return (
    <Dialog open onOpenChange={(open) => !open && onClose()} title={t("manage.title")}>
      <Tabs<ManageTab>
        className="mb-4"
        value={tab}
        onChange={setTab}
        items={tabs.map((value) => ({ value, label: t(`manage.tabs.${value}`) }))}
      />
      {tab === "edit" && <ChannelForm channel={channel} onSaved={onClose} onCancel={onClose} />}
      {tab === "moderators" && <Moderators channel={channel} onLeft={onClose} />}
      {tab === "archive" && <ArchiveSection channel={channel} onDone={onClose} />}
    </Dialog>
  );
}

function Moderators({ channel, onLeft }: { channel: notes.NoteChannel; onLeft(): void }) {
  const { t } = useTranslation("channels");
  const me = useMe();
  const queryClient = useQueryClient();
  const moderators = useChannelModerators(channel.id);
  const [handle, setHandle] = useState("");
  const [leaving, setLeaving] = useState(false);
  const reload = () =>
    Promise.all([
      queryClient.invalidateQueries({ queryKey: channelKeys.moderators(channel.id) }),
      queryClient.invalidateQueries({ queryKey: channelKeys.channel(channel.id) }),
      queryClient.invalidateQueries({ queryKey: channelKeys.lists }),
    ]);

  const add = useMutation({
    mutationFn: async () => {
      const value = handle.trim().replace(/^@/, "");
      const user = (await profileClient().getProfile({ by: { case: "handle", value } })).profile
        ?.user;
      if (!user) throw new Error(t("manage.userNotFound"));
      await channelClient().addChannelModerator({ channelId: channel.id, userId: user.id });
    },
    onSuccess: () => {
      setHandle("");
      toast.success(t("manage.added"));
      void reload();
    },
    onError: (error) =>
      toast.error(
        ["USER_NOT_FOUND", "PROFILE_NOT_FOUND"].includes(errorReason(error) ?? "")
          ? t("manage.userNotFound")
          : errorMessage(error, { namespace: "channels" }),
      ),
  });
  const remove = useMutation({
    mutationFn: (userId: string) =>
      channelClient().removeChannelModerator({ channelId: channel.id, userId }),
    onSuccess: (_, userId) => {
      void reload();
      if (userId === me?.user?.id) {
        toast.success(t("manage.stepDownDone"));
        onLeft();
      } else toast.success(t("manage.removed"));
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "channels" })),
  });

  return (
    <div className="flex flex-col gap-4">
      <p className="text-sm text-ink-soft">{t("manage.moderatorsLead")}</p>
      {channel.isOwner && (
        <form
          className="flex items-end gap-2"
          onSubmit={(event) => {
            event.preventDefault();
            if (handle.trim()) add.mutate();
          }}
        >
          <TextField
            className="flex-1"
            label={t("manage.addLabel")}
            placeholder={t("manage.addPlaceholder")}
            value={handle}
            onChange={(event) => setHandle(event.target.value)}
          />
          <Button
            type="submit"
            icon={<UserPlus className="size-4" />}
            loading={add.isPending}
            disabled={!handle.trim()}
          >
            {t("manage.add")}
          </Button>
        </form>
      )}
      {moderators.isPending ? (
        <Skeleton className="h-14 rounded-2xl" />
      ) : moderators.data?.length ? (
        <ul className="flex flex-col gap-1" aria-label={t("manage.tabs.moderators")}>
          {moderators.data.map((user) => (
            <li key={user.id} className="flex items-center gap-3 rounded-2xl px-2 py-2">
              <Avatar user={user} size="sm" />
              <span className="min-w-0 flex-1">
                <span className="block truncate font-bold">{user.displayName}</span>
                <span className="block truncate text-xs text-ink-faint">@{user.handle}</span>
              </span>
              {channel.isOwner && (
                <Button
                  variant="ghost"
                  size="sm"
                  loading={remove.isPending && remove.variables === user.id}
                  onClick={() => remove.mutate(user.id)}
                >
                  {t("manage.remove")}
                </Button>
              )}
            </li>
          ))}
        </ul>
      ) : (
        <p className="rounded-2xl bg-surface-2 px-4 py-3 text-sm font-bold text-ink-soft">
          {t("manage.moderatorsEmpty")}
        </p>
      )}
      {channel.isModerator && !channel.isOwner && (
        <Button variant="secondary" onClick={() => setLeaving(true)}>
          {t("manage.stepDown")}
        </Button>
      )}
      <ConfirmDialog
        open={leaving}
        onOpenChange={setLeaving}
        title={t("manage.stepDownTitle")}
        description={t("manage.stepDownBody")}
        confirmLabel={t("manage.stepDown")}
        danger
        loading={remove.isPending}
        onConfirm={() => me?.user && remove.mutate(me.user.id)}
      />
    </div>
  );
}

function ArchiveSection({ channel, onDone }: { channel: notes.NoteChannel; onDone(): void }) {
  const { t } = useTranslation("channels");
  const queryClient = useQueryClient();
  const [confirming, setConfirming] = useState(false);
  const archive = useMutation({
    mutationFn: async (archived: boolean) =>
      (await channelClient().archiveChannel({ channelId: channel.id, archived })).channel!,
    onSuccess: (updated) => {
      storeChannel(queryClient, updated);
      toast.success(t(updated.archived ? "manage.archivedDone" : "manage.unarchivedDone"));
      onDone();
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "channels" })),
  });
  return (
    <div className="flex flex-col gap-4">
      <p className="text-sm text-ink-soft">{t("manage.archiveLead")}</p>
      {channel.archived ? (
        <Button variant="lime" loading={archive.isPending} onClick={() => archive.mutate(false)}>
          {t("manage.unarchive")}
        </Button>
      ) : (
        <Button variant="danger" onClick={() => setConfirming(true)}>
          {t("manage.archive")}
        </Button>
      )}
      <ConfirmDialog
        open={confirming}
        onOpenChange={setConfirming}
        title={t("manage.archiveTitle")}
        description={t("manage.archiveBody")}
        confirmLabel={t("manage.archive")}
        danger
        loading={archive.isPending}
        onConfirm={() => archive.mutate(true)}
      />
    </div>
  );
}
