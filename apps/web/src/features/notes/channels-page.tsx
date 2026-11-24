import { useMutation, useQueryClient } from "@tanstack/react-query";
import type { notes } from "@tokuly/api";
import { Archive, Pin, PinOff, Settings, SquarePen, Trash2 } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { errorMessage } from "#/lib/errors";
import { useMe } from "#/stores/session";
import {
  channelClient,
  channelKeys,
  storeChannel,
  useChannel,
  useChannelTimeline,
  useRefreshChannel,
} from "./channels-api";
import { ChannelDot } from "./channels-chip";
import { ChannelManageDialog } from "./channels-settings";
import { openCompose, useCompose } from "./compose-store";
import { imageUrl } from "./media";
import { NoteCard } from "./note-card";

/** A channel: its banner and description, follow, pinned notes, its notes and a way to post in. */
export function ChannelPage({ channelId }: { channelId: string }) {
  const { t } = useTranslation("channels");
  const me = useMe();
  const queryClient = useQueryClient();
  const channel = useChannel(channelId);
  const timeline = useChannelTimeline(channelId);
  const [managing, setManaging] = useState(false);

  // A note posted from the composer opened here joins the top of the channel.
  useEffect(
    () =>
      useCompose.subscribe((now, before) => {
        if (before.open && !now.open && before.context.channelId === channelId) {
          void queryClient.invalidateQueries({ queryKey: channelKeys.timeline(channelId) });
          void queryClient.invalidateQueries({ queryKey: channelKeys.channel(channelId) });
        }
      }),
    [channelId, queryClient],
  );

  const follow = useMutation({
    mutationFn: async (following: boolean) => {
      if (following) return (await channelClient().followChannel({ channelId })).channel!;
      await channelClient().unfollowChannel({ channelId });
      return (await channelClient().getChannel({ channelId })).channel!;
    },
    onSuccess: (updated, following) => {
      storeChannel(queryClient, updated);
      toast.success(t(following ? "followed" : "unfollowed"));
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "channels" })),
  });

  if (channel.isError) {
    return (
      <Page title={t("title")} back="/notes/channels">
        <EmptyState emoji="📺" title={t("notFound")} body={t("notFoundBody")} />
      </Page>
    );
  }
  const data = channel.data;
  const signedIn = Boolean(me?.user);
  const manager = Boolean(data && (data.isOwner || data.isModerator));
  const pinned = timeline.data?.pages[0]?.pinned ?? [];
  const entries = timeline.data?.pages.flatMap((page) => page.entries) ?? [];
  const canPost = signedIn && data && !data.archived;

  return (
    <Page
      title={data?.name ?? t("title")}
      back="/notes/channels"
      actions={
        manager && (
          <Button
            variant="ghost"
            size="icon-sm"
            aria-label={t("settings")}
            onClick={() => setManaging(true)}
          >
            <Settings className="size-5" />
          </Button>
        )
      }
    >
      {!data ? (
        <Skeleton className="h-48 rounded-card" />
      ) : (
        <section className="sticker overflow-hidden rounded-card bg-surface">
          <div
            className="h-28 bg-surface-2 bg-cover bg-center"
            style={{
              backgroundColor: data.color || undefined,
              backgroundImage: data.banner ? `url(${imageUrl(data.banner, 960)})` : undefined,
            }}
          />
          <div className="p-4">
            <h2 className="flex items-center gap-2 text-xl font-black">
              <ChannelDot color={data.color} className="size-3.5" />
              <span className="min-w-0 truncate">{data.name}</span>
            </h2>
            {data.description && (
              <p className="mt-1.5 whitespace-pre-wrap text-sm text-ink-soft">{data.description}</p>
            )}
            <p className="mt-2 flex flex-wrap gap-3 text-xs font-bold text-ink-faint">
              <span>{t("followers", { count: Number(data.followerCount) })}</span>
              <span>{t("noteCount", { count: Number(data.noteCount) })}</span>
              {data.owner && (
                <span>
                  {t("owner")}: @{data.owner.handle}
                </span>
              )}
            </p>
            {signedIn && (
              <div className="mt-3 flex flex-wrap gap-2">
                <Button
                  size="sm"
                  variant={data.following ? "secondary" : "lime"}
                  loading={follow.isPending}
                  aria-pressed={data.following}
                  onClick={() => follow.mutate(!data.following)}
                >
                  {data.following ? t("following") : t("follow")}
                </Button>
                {canPost && (
                  <Button
                    size="sm"
                    icon={<SquarePen className="size-4" />}
                    onClick={() => openCompose({ channelId })}
                  >
                    {t("post")}
                  </Button>
                )}
              </div>
            )}
          </div>
        </section>
      )}

      {data?.archived && (
        <p className="mt-3 flex items-start gap-2 rounded-2xl bg-surface-2 px-4 py-3 text-sm font-bold text-ink-soft">
          <Archive className="mt-0.5 size-4 shrink-0" />
          {t("archivedNotice")}
        </p>
      )}

      {pinned.length > 0 && (
        <section className="mt-5" aria-label={t("pinned")}>
          <h3 className="mb-2 flex items-center gap-1.5 text-sm font-black text-brand">
            <Pin className="size-4 fill-current" />
            {t("pinned")}
          </h3>
          <div className="flex flex-col gap-2.5">
            {pinned.map((note) => (
              <ChannelNote key={note.id} note={note} channel={data} manager={manager} pinned />
            ))}
          </div>
        </section>
      )}

      <div className="mt-5 flex flex-col gap-2.5" data-testid="channel-timeline">
        {timeline.isPending &&
          Array.from({ length: 3 }, (_, index) => (
            <Skeleton key={index} className="h-28 rounded-[1.25rem]" />
          ))}
        {!timeline.isPending && entries.length === 0 && (
          <EmptyState
            emoji="🌱"
            title={t("timelineEmpty")}
            body={canPost ? t("timelineEmptyBody") : undefined}
            action={
              canPost ? (
                <Button onClick={() => openCompose({ channelId })}>{t("post")}</Button>
              ) : undefined
            }
          />
        )}
        {entries.map((entry) =>
          entry.note ? (
            <ChannelNote
              key={entry.note.id}
              note={entry.note}
              channel={data}
              manager={manager}
              pinned={data?.pinnedNoteIds.includes(entry.note.id) ?? false}
            />
          ) : null,
        )}
        <LoadMore query={timeline} showEnd={entries.length > 0} />
      </div>
      {managing && data && (
        <ChannelManageDialog channel={data} onClose={() => setManaging(false)} />
      )}
    </Page>
  );
}

/** A note of the channel, with pin and take-out buttons for the channel's moderators. */
function ChannelNote({
  note,
  channel,
  manager,
  pinned,
}: {
  note: notes.Note;
  channel: notes.NoteChannel | undefined;
  manager: boolean;
  pinned: boolean;
}) {
  const { t } = useTranslation("channels");
  const [removing, setRemoving] = useState(false);
  const channelId = channel?.id ?? "";
  const refresh = useRefreshChannel(channelId);

  const pin = useMutation({
    mutationFn: async (pin: boolean) => {
      if (pin) await channelClient().pinChannelNote({ channelId, noteId: note.id });
      else await channelClient().unpinChannelNote({ channelId, noteId: note.id });
    },
    onSuccess: (_, pin) => {
      toast.success(t(pin ? "mod.pinnedDone" : "mod.unpinnedDone"));
      void refresh();
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "channels" })),
  });
  const remove = useMutation({
    mutationFn: () => channelClient().removeChannelNote({ channelId, noteId: note.id }),
    onSuccess: () => {
      setRemoving(false);
      toast.success(t("mod.removedDone"));
      void refresh();
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "channels" })),
  });

  return (
    <div>
      <NoteCard note={note} />
      {manager && (
        <div
          role="toolbar"
          aria-label={t("mod.label")}
          className="mt-1 flex justify-end gap-1 px-1"
        >
          <Button
            variant="ghost"
            size="sm"
            icon={pinned ? <PinOff className="size-4" /> : <Pin className="size-4" />}
            loading={pin.isPending}
            onClick={() => pin.mutate(!pinned)}
          >
            {pinned ? t("mod.unpin") : t("mod.pin")}
          </Button>
          <Button
            variant="ghost"
            size="sm"
            icon={<Trash2 className="size-4" />}
            onClick={() => setRemoving(true)}
          >
            {t("mod.remove")}
          </Button>
        </div>
      )}
      <ConfirmDialog
        open={removing}
        onOpenChange={setRemoving}
        title={t("mod.removeTitle")}
        description={t("mod.removeBody")}
        confirmLabel={t("mod.remove")}
        danger
        loading={remove.isPending}
        onConfirm={() => remove.mutate()}
      />
    </div>
  );
}
