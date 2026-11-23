import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { common, errorReason, live } from "@tokuly/api";
import { Film, Loader, MoreHorizontal, Pencil, Radio, Trash2, TriangleAlert } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { Dialog } from "#/components/ui/dialog";
import { TextField } from "#/components/ui/field";
import { Menu } from "#/components/ui/menu";
import { EmptyState, Page } from "#/components/ui/page";
import { PaneSpinner } from "#/components/ui/spinner";
import { EmojiText } from "#/features/emoji";
import { LikeButton, ShareButton, useSubjectMenu } from "#/features/engagement";
import { VideoPlayer } from "#/features/video";
import { errorMessage } from "#/lib/errors";
import { useMe } from "#/stores/session";
import { clipClient, liveKeys } from "../api";
import { ChannelFace, ChannelTitle, formatMoment } from "../parts";
import { clock } from "../player/live-player";
import { ClipStrip } from "./clip-list";

const { ClipStatus } = live;

/** One clip; `null` when it does not exist or its source is not the caller's to see. */
export function useClip(clipId: string) {
  return useQuery({
    queryKey: liveKeys.clip(clipId),
    queryFn: async () => {
      try {
        return (await clipClient().getClip({ clipId })).clip ?? null;
      } catch (error) {
        if (errorReason(error) === "CLIP_NOT_FOUND") return null;
        throw error;
      }
    },
  });
}

/** `/clip/<id>`: a clip playing, who cut it from what, and the way to the whole thing. */
export function ClipScreen({ clipId }: { clipId: string }) {
  const { t } = useTranslation("live");
  const query = useClip(clipId);
  if (query.isPending) return <PaneSpinner />;
  return (
    <Page title={t("clip.pageTitle")} back="/live">
      {query.data ? (
        <ClipView clip={query.data} />
      ) : (
        <EmptyState emoji="✂️" title={t("clip.notFoundTitle")} body={t("clip.notFoundBody")} />
      )}
    </Page>
  );
}

function ClipView({ clip }: { clip: live.Clip }) {
  const { t, i18n } = useTranslation("live");
  const me = useMe();
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const mine = clip.creator?.id === me?.user?.id;
  const subject = { type: common.SubjectType.CLIP, id: clip.id };
  const more = useSubjectMenu(subject, { title: clip.title, label: clip.title, report: !mine });
  const [renaming, setRenaming] = useState(false);
  const [title, setTitle] = useState(clip.title);
  const [deleting, setDeleting] = useState(false);

  const store = (next: live.Clip | undefined) => {
    if (!next) return;
    queryClient.setQueryData(liveKeys.clip(next.shortId), next);
    queryClient.setQueryData(liveKeys.clip(next.id), next);
    void queryClient.invalidateQueries({ queryKey: ["live", "clip", "list"] });
  };
  const rename = useMutation({
    mutationFn: () => clipClient().updateClip({ clipId: clip.id, title: title.trim() }),
    onSuccess: (response) => {
      store(response.clip);
      setRenaming(false);
      toast.success(t("clip.renamed"));
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "live" })),
  });
  const remove = useMutation({
    mutationFn: () => clipClient().deleteClip({ clipId: clip.id }),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: liveKeys.clips });
      toast.success(t("clip.deleted"));
      void navigate({ to: "/live" });
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "live" })),
  });

  const actions = [
    ...(mine
      ? [
          {
            label: t("clip.rename"),
            icon: <Pencil />,
            onSelect: () => {
              setTitle(clip.title);
              setRenaming(true);
            },
          },
        ]
      : []),
    ...(clip.canDelete
      ? [
          {
            label: t("clip.delete"),
            icon: <Trash2 />,
            danger: true,
            onSelect: () => setDeleting(true),
          },
        ]
      : []),
    ...(clip.status === ClipStatus.READY ? more.actions : []),
  ];

  return (
    <div data-testid="clip-page" data-status={clip.status}>
      {clip.status === ClipStatus.READY && clip.playback ? (
        <VideoPlayer source={clip.playback} title={clip.title} autoPlay shortcuts />
      ) : (
        <div className="grid aspect-video w-full place-items-center rounded-2xl bg-surface-3 p-6 text-center">
          {clip.status === ClipStatus.FAILED ? (
            <div className="flex flex-col items-center gap-2 text-ink-soft">
              <TriangleAlert className="size-8" />
              <p className="font-black">{t("clip.failedTitle")}</p>
              <p className="text-sm">{t("clip.failedBody")}</p>
            </div>
          ) : (
            <div className="flex flex-col items-center gap-2 text-ink-soft" role="status">
              <Loader className="size-8 animate-spin" />
              <p className="font-black">{t("clip.processingTitle")}</p>
              <p className="text-sm">{t("clip.processingBody")}</p>
            </div>
          )}
        </div>
      )}
      <div className="mt-3 flex items-start gap-2">
        <h2 className="min-w-0 flex-1 text-xl leading-snug font-black">
          <EmojiText text={clip.title} />
        </h2>
        {actions.length > 0 && (
          <Menu
            trigger={
              <button
                type="button"
                aria-label={t("clip.more")}
                className="grid size-9 shrink-0 place-items-center rounded-full text-ink-soft hover:bg-surface-2 hover:text-ink"
              >
                <MoreHorizontal className="size-5" />
              </button>
            }
            actions={actions}
          />
        )}
        {more.dialogs}
      </div>
      <p className="mt-1 text-sm font-bold text-ink-soft">
        {clip.createdAt && formatMoment(Number(clip.createdAt.seconds) * 1000, i18n.language)}
        {" · "}
        {t("clip.seconds", { seconds: clip.durationSeconds.toFixed(1) })}
      </p>
      {clip.status === ClipStatus.READY && (
        <div className="mt-2 flex items-center gap-1.5">
          <LikeButton subject={subject} />
          <ShareButton subject={subject} title={clip.title} />
        </div>
      )}

      <div className="mt-4 flex flex-col gap-2 rounded-2xl bg-surface-2 p-3">
        {clip.creator && (
          <Link
            to="/u/$handle"
            params={{ handle: clip.creator.handle }}
            className="flex items-center gap-2.5"
          >
            <Avatar user={clip.creator} size="sm" />
            <span className="min-w-0 text-sm">
              <span className="block text-xs font-bold text-ink-faint">{t("clip.cutBy")}</span>
              <span className="block truncate font-black">
                <EmojiText text={clip.creator.displayName} />
              </span>
            </span>
          </Link>
        )}
        {clip.channel && (
          <Link
            to="/c/$handle"
            params={{ handle: clip.channel.handle }}
            className="flex items-center gap-2.5"
          >
            <ChannelFace channel={clip.channel} />
            <span className="min-w-0 text-sm">
              <span className="block text-xs font-bold text-ink-faint">{t("clip.cutFrom")}</span>
              <ChannelTitle channel={clip.channel} className="font-black" />
            </span>
          </Link>
        )}
        <div className="mt-1 flex flex-wrap gap-2">
          {clip.videoId ? (
            <Link
              to="/watch/$videoId"
              params={{ videoId: clip.videoId }}
              search={{ t: String(Math.floor(clip.startSeconds)) }}
              className="pressable sticker-sm inline-flex h-9 items-center gap-1.5 rounded-full bg-surface px-4 text-sm font-black"
            >
              <Film className="size-4" />
              {t("clip.watchSource", { clock: clock(clip.startSeconds) })}
            </Link>
          ) : (
            clip.streamId && (
              <Link
                to="/live/$streamId"
                params={{ streamId: clip.streamId }}
                className="pressable sticker-sm inline-flex h-9 items-center gap-1.5 rounded-full bg-surface px-4 text-sm font-black"
              >
                <Radio className="size-4" />
                {t("clip.watchStream")}
              </Link>
            )
          )}
        </div>
        {clip.sourceTitle && (
          <p className="truncate text-xs font-bold text-ink-faint">
            <EmojiText text={clip.sourceTitle} />
          </p>
        )}
      </div>
      {clip.channel && <ClipStrip kind="channel" id={clip.channel.id} className="mt-5" />}

      <Dialog
        open={renaming}
        onOpenChange={setRenaming}
        title={t("clip.rename")}
        footer={
          <Button
            loading={rename.isPending}
            disabled={!title.trim()}
            onClick={() => rename.mutate()}
          >
            {t("common:actions.save")}
          </Button>
        }
      >
        <TextField
          label={t("clip.titleLabel")}
          value={title}
          maxLength={64}
          onChange={(event) => setTitle(event.target.value)}
        />
      </Dialog>
      <ConfirmDialog
        open={deleting}
        onOpenChange={setDeleting}
        title={t("clip.deleteTitle")}
        description={t("clip.deleteBody")}
        confirmLabel={t("clip.delete")}
        danger
        loading={remove.isPending}
        onConfirm={() => remove.mutate()}
      />
    </div>
  );
}
