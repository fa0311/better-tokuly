import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { common, video } from "@tokuly/api";
import { playlistTitle } from "@tokuly/core/video";
import { Check, Clock3, ListVideo, Plus } from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Dialog } from "#/components/ui/dialog";
import { TextField } from "#/components/ui/field";
import { Skeleton } from "#/components/ui/page";
import { SelectField } from "#/components/ui/select";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { useTranslation } from "#/lib/use-translation";
import { playlistClient, videoKeys } from "./api";
import { PLAYLIST_VISIBILITIES } from "./labels";
import { useSaveTarget } from "./save-target";

const { PlaylistKind } = video;

/**
 * The one "save to…" dialog of the app: the viewer's playlists with a tick
 * on those that hold the video, and a way to start a new one. It is mounted
 * once (by the feature's background component) and opened with `openSaveDialog`.
 */
export function SaveDialogHost() {
  const { t } = useTranslation("video");
  const target = useSaveTarget((state) => state.video);
  // The last video stays on screen while the dialog closes.
  const [shown, setShown] = useState<video.VideoSummary | null>(null);
  if (target && target !== shown) setShown(target);

  return (
    <Dialog
      open={target !== null}
      onOpenChange={(open) => !open && useSaveTarget.setState({ video: null })}
      title={t("save.title")}
      description={shown?.title}
    >
      {shown && <SaveBody key={shown.id} video={shown} />}
    </Dialog>
  );
}

function SaveBody({ video: summary }: { video: video.VideoSummary }) {
  const { t } = useTranslation("video");
  const queryClient = useQueryClient();
  const [creating, setCreating] = useState(false);
  const key = videoKeys.myPlaylists(summary.id);
  const lists = useQuery({
    queryKey: key,
    queryFn: () => playlistClient().listMyPlaylists({ containsVideoId: summary.id }),
  });

  const toggle = useMutation({
    mutationFn: async ({ playlist, add }: { playlist: video.Playlist; add: boolean }) => {
      if (add)
        await playlistClient().addPlaylistItem({ playlistId: playlist.id, videoId: summary.id });
      else
        await playlistClient().removePlaylistItem({ playlistId: playlist.id, videoId: summary.id });
    },
    // The tick answers the tap; a refusal puts it back.
    onMutate: ({ playlist, add }) => setContains(playlist.id, add),
    onSuccess: (_result, { playlist, add }) => {
      toast.success(
        add
          ? t("save.added", { name: playlistTitle(playlist, t) })
          : t("save.removed", { name: playlistTitle(playlist, t) }),
      );
      void queryClient.invalidateQueries({ queryKey: videoKeys.playlists });
      void queryClient.invalidateQueries({ queryKey: videoKeys.video(summary.shortId) });
    },
    onError: (error, { playlist, add }) => {
      setContains(playlist.id, !add);
      toast.error(errorMessage(error, { namespace: "video" }));
    },
  });

  function setContains(playlistId: string, containsVideo: boolean) {
    queryClient.setQueryData<video.ListMyPlaylistsResponse>(key, (data) =>
      data
        ? {
            ...data,
            playlists: data.playlists.map((playlist) =>
              playlist.id === playlistId ? { ...playlist, containsVideo } : playlist,
            ),
          }
        : data,
    );
  }

  // Liked videos change through the like button only.
  const playlists =
    lists.data?.playlists.filter((playlist) => playlist.kind !== PlaylistKind.LIKED) ?? [];

  return (
    <div className="flex flex-col gap-1">
      {lists.isPending ? (
        [0, 1, 2].map((row) => <Skeleton key={row} className="h-12" />)
      ) : lists.isError ? (
        <p className="py-6 text-center text-sm font-bold text-ink-soft">
          {errorMessage(lists.error, { namespace: "video" })}
        </p>
      ) : (
        playlists.map((playlist) => (
          // biome-ignore lint/a11y/useSemanticElements: a row-sized tick; a native checkbox cannot carry this layout.
          <button
            key={playlist.id}
            type="button"
            role="checkbox"
            aria-checked={playlist.containsVideo}
            className="flex min-h-12 items-center gap-3 rounded-2xl px-3 text-left hover:bg-surface-2"
            onClick={() => toggle.mutate({ playlist, add: !playlist.containsVideo })}
          >
            <span
              className={cn(
                "grid size-6 shrink-0 place-items-center rounded-lg border-2 border-ink",
                playlist.containsVideo ? "bg-lime text-[#17122b]" : "bg-surface",
              )}
            >
              {playlist.containsVideo && <Check className="size-4" strokeWidth={3} />}
            </span>
            <span className="min-w-0 flex-1 truncate font-bold">{playlistTitle(playlist, t)}</span>
            {playlist.kind === PlaylistKind.WATCH_LATER ? (
              <Clock3 className="size-4 text-ink-faint" />
            ) : (
              <ListVideo className="size-4 text-ink-faint" />
            )}
            <span className="text-xs font-bold text-ink-faint tabular-nums">
              {playlist.itemCount}
            </span>
          </button>
        ))
      )}

      {creating ? (
        <NewPlaylistForm videoId={summary.id} onDone={() => setCreating(false)} />
      ) : (
        <Button
          variant="secondary"
          className="mt-3 self-start"
          icon={<Plus className="size-4" />}
          onClick={() => setCreating(true)}
        >
          {t("save.newPlaylist")}
        </Button>
      )}
    </div>
  );
}

/** Starts a new playlist, with the video already in it. */
function NewPlaylistForm({ videoId, onDone }: { videoId: string; onDone(): void }) {
  const { t } = useTranslation("video");
  const queryClient = useQueryClient();
  const [title, setTitle] = useState("");
  const [visibility, setVisibility] = useState<common.Visibility>(common.Visibility.PRIVATE);

  const create = useMutation({
    mutationFn: () => playlistClient().createPlaylist({ title: title.trim(), visibility, videoId }),
    onSuccess: ({ playlist }) => {
      toast.success(t("save.added", { name: playlist?.title ?? title }));
      void queryClient.invalidateQueries({ queryKey: videoKeys.playlists });
      onDone();
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "video" })),
  });

  return (
    <form
      className="mt-3 flex flex-col gap-3 rounded-2xl bg-surface-2 p-3"
      onSubmit={(event) => {
        event.preventDefault();
        if (title.trim()) create.mutate();
      }}
    >
      <TextField
        label={t("playlist.form.title")}
        value={title}
        maxLength={100}
        autoFocus
        onChange={(event) => setTitle(event.target.value)}
        placeholder={t("playlist.form.titlePlaceholder")}
      />
      <SelectField
        label={t("playlist.form.visibility")}
        value={String(visibility)}
        onChange={(value) => setVisibility(Number(value) as common.Visibility)}
        options={PLAYLIST_VISIBILITIES.map((option) => ({
          value: String(option.value),
          label: t(`visibility.${option.key}`),
          hint: t(`visibility.${option.key}ListHint`),
        }))}
      />
      <div className="flex justify-end gap-2">
        <Button variant="ghost" onClick={onDone}>
          {t("actions.cancel")}
        </Button>
        <Button type="submit" disabled={!title.trim()} loading={create.isPending}>
          {t("save.create")}
        </Button>
      </div>
    </form>
  );
}
