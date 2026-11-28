import { useMutation, useQueryClient } from "@tanstack/react-query";
import { common, type video } from "@tokuly/api";
import { useState } from "react";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Dialog } from "#/components/ui/dialog";
import { TextArea, TextField } from "#/components/ui/field";
import { SelectField } from "#/components/ui/select";
import { errorMessage } from "#/lib/errors";
import { useTranslation } from "#/lib/use-translation";
import { playlistClient, useMyChannels, videoKeys } from "../api";
import { PLAYLIST_VISIBILITIES } from "../labels";

const PERSONAL = "personal";

/**
 * Creates a playlist, or — given one — edits its name, description, audience
 * and whether it is published on one of the owner's channels.
 */
export function PlaylistFormDialog({
  open,
  onOpenChange,
  playlist,
  onSaved,
}: {
  open: boolean;
  onOpenChange(open: boolean): void;
  playlist?: video.Playlist;
  onSaved?(playlist: video.Playlist): void;
}) {
  const { t } = useTranslation("video");
  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={playlist ? t("playlist.form.editTitle") : t("playlist.form.newTitle")}
    >
      {open && (
        <PlaylistForm
          playlist={playlist}
          onCancel={() => onOpenChange(false)}
          onSaved={(saved) => (onSaved ? onSaved(saved) : onOpenChange(false))}
        />
      )}
    </Dialog>
  );
}

function PlaylistForm({
  playlist,
  onCancel,
  onSaved,
}: {
  playlist?: video.Playlist;
  onCancel(): void;
  onSaved(playlist: video.Playlist): void;
}) {
  const { t } = useTranslation("video");
  const queryClient = useQueryClient();
  const channels = useMyChannels();
  const [title, setTitle] = useState(playlist?.title ?? "");
  const [description, setDescription] = useState(playlist?.description ?? "");
  const [visibility, setVisibility] = useState<common.Visibility>(
    playlist?.visibility || common.Visibility.PRIVATE,
  );
  const [channelId, setChannelId] = useState(playlist?.channel?.id ?? PERSONAL);

  const save = useMutation({
    mutationFn: async () => {
      const fields = {
        title: title.trim(),
        description: description.trim(),
        visibility,
        channelId: channelId === PERSONAL ? "" : channelId,
      };
      const response = playlist
        ? await playlistClient().updatePlaylist({ playlistId: playlist.id, ...fields })
        : await playlistClient().createPlaylist(fields);
      return response.playlist;
    },
    onSuccess: (saved) => {
      void queryClient.invalidateQueries({ queryKey: videoKeys.playlists });
      void queryClient.invalidateQueries({ queryKey: ["video", "channel", "playlists"] });
      toast.success(playlist ? t("playlist.toast.saved") : t("playlist.toast.created"));
      if (saved) onSaved(saved);
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "video" })),
  });
  const owned =
    channels.data?.channels.flatMap((channel) => (channel.summary ? [channel.summary] : [])) ?? [];

  return (
    <form
      className="flex flex-col gap-4"
      onSubmit={(event) => {
        event.preventDefault();
        if (title.trim()) save.mutate();
      }}
    >
      <TextField
        label={t("playlist.form.title")}
        value={title}
        maxLength={100}
        autoFocus
        placeholder={t("playlist.form.titlePlaceholder")}
        onChange={(event) => setTitle(event.target.value)}
      />
      <TextArea
        label={t("playlist.form.description")}
        value={description}
        maxLength={1000}
        placeholder={t("playlist.form.descriptionPlaceholder")}
        onChange={(event) => setDescription(event.target.value)}
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
      {owned.length > 0 && (
        <SelectField
          label={t("playlist.form.channel")}
          hint={t("playlist.form.channelHint")}
          value={channelId}
          onChange={setChannelId}
          options={[
            { value: PERSONAL, label: t("playlist.form.personal") },
            ...owned.map((channel) => ({
              value: channel.id,
              label: channel.name,
              hint: `@${channel.handle}`,
            })),
          ]}
        />
      )}
      <div className="flex justify-end gap-2 pb-safe">
        <Button variant="ghost" onClick={onCancel}>
          {t("actions.cancel")}
        </Button>
        <Button type="submit" disabled={!title.trim()} loading={save.isPending}>
          {playlist ? t("actions.save") : t("save.create")}
        </Button>
      </div>
    </form>
  );
}
