import { useMutation, useQueryClient } from "@tanstack/react-query";
import { community } from "@tokuly/api";
import { communityKeys } from "@tokuly/core/community";
import { GripVertical, Lock, Pencil, Plus, Trash2, VenetianMask } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Chip } from "#/components/ui/card";
import { ConfirmDialog } from "#/components/ui/confirm";
import { Dialog } from "#/components/ui/dialog";
import { TextField } from "#/components/ui/field";
import { Skeleton } from "#/components/ui/page";
import { Sortable } from "#/components/ui/sortable";
import { SwitchRow } from "#/components/ui/switch";
import { EmojiPicker } from "#/features/emoji";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { boardClient, NS, useChannels } from "../api";
import { Notice } from "../parts";

const MODERATORS = community.PostingPolicy.MODERATORS;

/** The channels of a community: add, edit, put in order, delete. */
export function ChannelsSection({ community: c }: { community: community.Community }) {
  const { t } = useTranslation(NS);
  const queryClient = useQueryClient();
  const channels = useChannels(c.id);
  const [editing, setEditing] = useState<community.Channel | "new">();
  const [deleting, setDeleting] = useState<community.Channel>();
  const refresh = () =>
    void queryClient.invalidateQueries({ queryKey: communityKeys.channels(c.id) });
  const onError = (error: unknown) => toast.error(errorMessage(error, { namespace: NS }));

  const reorder = useMutation({
    mutationFn: (ordered: community.Channel[]) =>
      boardClient().reorderChannels({
        communityId: c.id,
        channelIds: ordered.map((channel) => channel.id),
      }),
    onMutate: (ordered) => queryClient.setQueryData(communityKeys.channels(c.id), ordered),
    onSuccess: ({ channels: next }) => queryClient.setQueryData(communityKeys.channels(c.id), next),
    onError: (error) => {
      refresh();
      onError(error);
    },
  });
  const remove = useMutation({
    mutationFn: (channelId: string) => boardClient().deleteChannel({ channelId }),
    onSuccess: () => {
      setDeleting(undefined);
      refresh();
      void queryClient.invalidateQueries({ queryKey: communityKeys.threadLists(c.id) });
      toast(t("manage.channels.deleted"));
    },
    onError,
  });

  if (channels.isPending) return <Skeleton className="h-40" />;
  const list = channels.data ?? [];
  return (
    <div className="flex flex-col gap-3">
      <Button
        className="self-start"
        icon={<Plus className="size-4" />}
        onClick={() => setEditing("new")}
      >
        {t("manage.channels.add")}
      </Button>
      <Sortable
        items={list}
        getKey={(channel) => channel.id}
        onReorder={(ordered) => reorder.mutate(ordered)}
        className="flex flex-col gap-2"
      >
        {(channel, { grip, dragging }) => (
          <div
            className={cn(
              "flex items-center gap-2 rounded-card bg-surface p-3",
              dragging && "sticker",
            )}
          >
            <button
              type="button"
              {...grip}
              aria-label={t("manage.channels.move", { name: channel.name })}
              className="grid size-9 shrink-0 cursor-grab place-items-center rounded-full text-ink-faint hover:bg-surface-2"
            >
              <GripVertical className="size-5" />
            </button>
            <div className="min-w-0 flex-1">
              <p className="truncate font-bold">
                {channel.emoji} {channel.name}
              </p>
              <div className="mt-0.5 flex flex-wrap gap-1.5">
                <Chip>{t("manage.channels.threads", { count: channel.threadCount })}</Chip>
                {channel.anonymous && (
                  <Chip>
                    <VenetianMask className="size-3" />
                    {t("anonymous.chip")}
                  </Chip>
                )}
                {channel.posting === MODERATORS && (
                  <Chip>
                    <Lock className="size-3" />
                    {t("channel.readOnly")}
                  </Chip>
                )}
              </div>
            </div>
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label={t("manage.channels.edit", { name: channel.name })}
              onClick={() => setEditing(channel)}
            >
              <Pencil className="size-4" />
            </Button>
            <Button
              variant="ghost"
              size="icon-sm"
              disabled={list.length <= 1}
              aria-label={t("manage.channels.delete", { name: channel.name })}
              onClick={() => setDeleting(channel)}
            >
              <Trash2 className="size-4 text-danger" />
            </Button>
          </div>
        )}
      </Sortable>
      {editing && (
        <ChannelDialog
          communityId={c.id}
          channel={editing === "new" ? undefined : editing}
          onClose={() => setEditing(undefined)}
        />
      )}
      <ConfirmDialog
        open={Boolean(deleting)}
        onOpenChange={(open) => !open && setDeleting(undefined)}
        title={t("manage.channels.deleteTitle", { name: deleting?.name })}
        description={t("manage.channels.deleteBody")}
        confirmLabel={t("delete")}
        danger
        loading={remove.isPending}
        onConfirm={() => deleting && remove.mutate(deleting.id)}
      />
    </div>
  );
}

function ChannelDialog({
  communityId,
  channel,
  onClose,
}: {
  communityId: string;
  channel?: community.Channel;
  onClose(): void;
}) {
  const { t } = useTranslation(NS);
  const queryClient = useQueryClient();
  const [name, setName] = useState(channel?.name ?? "");
  const [description, setDescription] = useState(channel?.description ?? "");
  const [emoji, setEmoji] = useState(channel?.emoji ?? "💬");
  const [staffOnly, setStaffOnly] = useState(channel?.posting === MODERATORS);
  const [anonymous, setAnonymous] = useState(channel?.anonymous ?? false);
  // Anonymous mode is fixed once a channel has threads.
  const modeLocked = Boolean(channel && channel.threadCount > 0);
  const save = useMutation({
    mutationFn: async (): Promise<unknown> => {
      const fields = {
        name: name.trim(),
        description: description.trim(),
        emoji,
        posting: staffOnly ? MODERATORS : community.PostingPolicy.EVERYONE,
      };
      return channel
        ? boardClient().updateChannel({
            channelId: channel.id,
            ...fields,
            anonymous: modeLocked ? undefined : anonymous,
          })
        : boardClient().createChannel({ communityId, ...fields, anonymous });
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: communityKeys.channels(communityId) });
      toast.success(channel ? t("saved") : t("manage.channels.added"));
      onClose();
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: NS })),
  });
  return (
    <Dialog
      open
      onOpenChange={(open) => !open && onClose()}
      title={channel ? t("manage.channels.editTitle") : t("manage.channels.add")}
      footer={
        <Button loading={save.isPending} disabled={!name.trim()} onClick={() => save.mutate()}>
          {channel ? t("save") : t("manage.channels.addAction")}
        </Button>
      }
    >
      <div className="flex flex-col gap-4">
        <div className="flex items-end gap-2">
          <EmojiPicker
            unicodeOnly
            side="bottom"
            onPick={setEmoji}
            trigger={
              <button
                type="button"
                aria-label={t("manage.channels.emoji")}
                className="grid size-12 shrink-0 place-items-center rounded-control border-2 border-line bg-surface text-2xl"
              >
                {emoji || "＋"}
              </button>
            }
          />
          <TextField
            className="min-w-0 flex-1"
            label={t("manage.channels.name")}
            value={name}
            maxLength={30}
            onChange={(event) => setName(event.target.value)}
          />
        </div>
        <TextField
          label={t("manage.channels.description")}
          value={description}
          maxLength={200}
          onChange={(event) => setDescription(event.target.value)}
        />
        <div className="rounded-2xl bg-surface-2 p-1">
          <SwitchRow
            label={t("manage.channels.staffOnly")}
            hint={t("manage.channels.staffOnlyHint")}
            checked={staffOnly}
            onChange={setStaffOnly}
          />
          <SwitchRow
            label={t("manage.channels.anonymous")}
            hint={
              modeLocked ? t("manage.channels.anonymousLocked") : t("manage.channels.anonymousHint")
            }
            checked={anonymous}
            disabled={modeLocked}
            onChange={setAnonymous}
          />
        </div>
        {anonymous && <Notice>{t("anonymous.staffNote")}</Notice>}
      </div>
    </Dialog>
  );
}
