import { useQueryClient } from "@tanstack/react-query";
import { video } from "@tokuly/api";
import { playlistTitle, videoKeys } from "@tokuly/core/video";
import { Check, Plus } from "lucide-react-native";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { KeyboardAvoidingView, Modal, Pressable, Text, TextInput, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { create } from "zustand";
import { Spinner } from "#/components/page";
import { useTheme } from "#/lib/theme";
import { useCreatePlaylist, useMyPlaylists, useSaveTo } from "./data";

/** The video being saved; the sheet is open while one is set. */
const useSaveTarget = create<{ video: video.VideoSummary | null }>(() => ({ video: null }));

/** Opens "save to…" for a video, from any screen. */
export function openSaveSheet(summary: video.VideoSummary): void {
  useSaveTarget.setState({ video: summary });
}

/**
 * The app's one "save to…" sheet: the viewer's playlists with a tick on
 * those that hold the video, and a new private one. Mounted once, by the
 * feature's background component.
 */
export function SaveSheetHost() {
  const target = useSaveTarget((state) => state.video);
  const close = () => useSaveTarget.setState({ video: null });
  return (
    <Modal visible={target !== null} transparent animationType="slide" onRequestClose={close}>
      <Pressable accessibilityRole="none" className="flex-1 bg-black/40" onPress={close} />
      {target ? <SaveSheet summary={target} onDone={close} /> : null}
    </Modal>
  );
}

function SaveSheet({ summary, onDone }: { summary: video.VideoSummary; onDone(): void }) {
  const { t } = useTranslation("video");
  const { colors } = useTheme();
  const insets = useSafeAreaInsets();
  const queryClient = useQueryClient();
  const playlists = useMyPlaylists(summary.id);
  const saveTo = useSaveTo();
  const createPlaylist = useCreatePlaylist();
  const [title, setTitle] = useState("");
  const [busy, setBusy] = useState(false);

  const toggle = async (playlist: video.Playlist) => {
    const add = !playlist.containsVideo;
    const key = videoKeys.myPlaylists(summary.id);
    // The tick moves at once; a refusal reads the lists again.
    queryClient.setQueryData<video.Playlist[]>(key, (current) =>
      current?.map((entry) =>
        entry.id === playlist.id ? { ...entry, containsVideo: add } : entry,
      ),
    );
    if (!(await saveTo(playlist.id, summary, add)))
      void queryClient.invalidateQueries({ queryKey: key });
  };
  const start = async () => {
    if (!title.trim()) return;
    setBusy(true);
    if (await createPlaylist(title.trim(), summary.id)) onDone();
    setBusy(false);
  };

  return (
    <KeyboardAvoidingView behavior="padding">
      <View
        testID="video-save-sheet"
        className="max-h-[520px] gap-1 rounded-t-3xl bg-surface px-2 pt-4"
        style={{ paddingBottom: insets.bottom + 12 }}
      >
        <Text className="px-4 pb-2 text-lg font-black text-ink">{t("save.title")}</Text>
        {playlists.isPending ? (
          <Spinner />
        ) : (
          playlists.data
            ?.filter((playlist) => playlist.kind !== video.PlaylistKind.LIKED)
            .map((playlist) => (
              <Pressable
                key={playlist.id}
                testID={`save-to-${playlist.shortId || playlist.id}`}
                accessibilityRole="checkbox"
                accessibilityState={{ checked: playlist.containsVideo }}
                onPress={() => void toggle(playlist)}
                className="flex-row items-center gap-3 rounded-2xl px-4 py-3 active:bg-surface-2"
              >
                <View
                  className={`size-6 items-center justify-center rounded-md border-2 border-ink ${playlist.containsVideo ? "bg-brand" : ""}`}
                >
                  {playlist.containsVideo ? <Check color={colors["on-brand"]} size={16} /> : null}
                </View>
                <Text numberOfLines={1} className="flex-1 text-base font-bold text-ink">
                  {playlistTitle(playlist, t)}
                </Text>
              </Pressable>
            ))
        )}
        <View className="flex-row items-center gap-2 border-line border-t px-3 pt-3">
          <TextInput
            testID="save-new-title"
            value={title}
            maxLength={100}
            onChangeText={setTitle}
            placeholder={t("app.newPlaylistPlaceholder")}
            placeholderTextColor={colors["ink-faint"]}
            className="flex-1 rounded-full bg-surface-2 px-4 py-2.5 text-base text-ink"
          />
          <Pressable
            testID="save-new-create"
            accessibilityRole="button"
            accessibilityLabel={t("save.create")}
            disabled={!title.trim() || busy}
            onPress={() => void start()}
            className="flex-row items-center gap-1 rounded-full bg-brand px-4 py-2.5"
          >
            <Plus color={colors["on-brand"]} size={18} />
            <Text className="font-black text-on-brand">{t("save.create")}</Text>
          </Pressable>
        </View>
      </View>
    </KeyboardAvoidingView>
  );
}
