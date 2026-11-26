import { type common, media, type notes } from "@tokuly/api";
import { Image } from "expo-image";
import { useVideoPlayer, VideoView } from "expo-video";
import { Play, X } from "lucide-react-native";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Modal, Pressable, Text, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { SERVER_URL } from "#/lib/config";

const absolute = (url: string) => (url.startsWith("/") ? `${SERVER_URL}${url}` : url);

/** The smallest stored variant that still covers `pixels` on a 3x screen. */
export function imageSource(image: common.Image | undefined, pixels: number): string | undefined {
  if (!image) return undefined;
  const wanted = pixels * 3;
  const fitting = image.variants.filter((v) => v.size >= wanted).sort((a, b) => a.size - b.size)[0];
  const url = fitting?.url || image.url;
  return url ? absolute(url) : undefined;
}

/**
 * A note's photos (a grid of up to four on the card; the rest behind the
 * last tile) or its one video, as a poster that plays full screen. A
 * sensitive attachment stays covered until tapped.
 */
export function NoteMedia({ note }: { note: notes.Note }) {
  const { t } = useTranslation("notes");
  const [revealed, setRevealed] = useState(false);
  const [opened, setOpened] = useState<number>();
  const attachments = note.attachments.filter(
    (attachment) => attachment.status !== media.AssetStatus.FAILED,
  );
  if (attachments.length === 0) return null;
  const sensitive = !revealed && note.attachmentDetails.some((details) => details.sensitive);
  if (sensitive)
    return (
      <Pressable
        testID="note-media-sensitive"
        accessibilityRole="button"
        accessibilityLabel={t("media.revealSensitive")}
        onPress={() => setRevealed(true)}
        className="h-40 items-center justify-center rounded-2xl bg-surface-3"
      >
        <Text className="font-black text-ink-soft">🙈 {t("media.sensitive")}</Text>
      </Pressable>
    );

  const video = attachments.find((attachment) => attachment.kind === media.AssetKind.VIDEO);
  if (video) return <VideoTile attachment={video} />;

  const shown = attachments.slice(0, 4);
  const rest = attachments.length - shown.length;
  return (
    <View className="flex-row flex-wrap gap-1 overflow-hidden rounded-2xl">
      {shown.map((attachment, index) => {
        const alt = note.attachmentDetails.find((d) => d.assetId === attachment.assetId)?.altText;
        return (
          <Pressable
            key={attachment.assetId || index}
            accessibilityRole="imagebutton"
            accessibilityLabel={
              alt || t("media.open", { index: index + 1, count: attachments.length })
            }
            onPress={() => setOpened(index)}
            style={{
              width: shown.length === 1 ? "100%" : "49.5%",
              aspectRatio: shown.length === 1 ? 4 / 3 : 1,
            }}
          >
            <Image
              source={imageSource(attachment.image, 240) ?? absolute(attachment.url)}
              contentFit="cover"
              style={{
                width: "100%",
                height: "100%",
                backgroundColor: attachment.image?.dominantColor || "#e4def5",
              }}
            />
            {rest > 0 && index === shown.length - 1 && (
              <View className="absolute inset-0 items-center justify-center bg-black/50">
                <Text className="text-2xl font-black text-white">+{rest}</Text>
              </View>
            )}
          </Pressable>
        );
      })}
      <PhotoViewer attachments={attachments} index={opened} onClose={() => setOpened(undefined)} />
    </View>
  );
}

/** The photos full screen, one at a time; tap the sides to go back and on. */
function PhotoViewer({
  attachments,
  index,
  onClose,
}: {
  attachments: media.Attachment[];
  index: number | undefined;
  onClose(): void;
}) {
  const { t } = useTranslation("notes");
  const insets = useSafeAreaInsets();
  const [at, setAt] = useState(index ?? 0);
  const [shownFor, setShownFor] = useState(index);
  if (index !== shownFor) {
    setShownFor(index);
    setAt(index ?? 0);
  }
  const attachment = attachments[at];
  return (
    <Modal visible={index !== undefined} animationType="fade" onRequestClose={onClose}>
      <View className="flex-1 bg-black" testID="photo-viewer">
        {attachment && (
          <Image
            source={imageSource(attachment.image, 480) ?? absolute(attachment.url)}
            contentFit="contain"
            style={{ flex: 1 }}
          />
        )}
        <View className="absolute inset-0 flex-row">
          <Pressable
            accessibilityLabel={t("media.previous")}
            className="flex-1"
            onPress={() => setAt(Math.max(0, at - 1))}
          />
          <Pressable
            accessibilityLabel={t("media.next")}
            className="flex-1"
            onPress={() => setAt(Math.min(attachments.length - 1, at + 1))}
          />
        </View>
        <Pressable
          accessibilityRole="button"
          accessibilityLabel={t("media.close")}
          onPress={onClose}
          className="absolute right-4 rounded-full bg-black/60 p-2"
          style={{ top: insets.top + 8 }}
        >
          <X color="#ffffff" size={22} />
        </Pressable>
        {attachments.length > 1 && (
          <Text
            className="absolute self-center font-black text-white"
            style={{ bottom: insets.bottom + 16 }}
          >
            {at + 1} / {attachments.length}
          </Text>
        )}
      </View>
    </Modal>
  );
}

function VideoTile({ attachment }: { attachment: media.Attachment }) {
  const [playing, setPlaying] = useState(false);
  const poster = imageSource(attachment.video?.poster, 320);
  return (
    <>
      <Pressable
        accessibilityRole="button"
        onPress={() => setPlaying(true)}
        className="aspect-video items-center justify-center overflow-hidden rounded-2xl bg-black"
      >
        {poster && (
          <Image source={poster} contentFit="cover" style={{ position: "absolute", inset: 0 }} />
        )}
        <View className="rounded-full bg-black/60 p-3">
          <Play color="#ffffff" fill="#ffffff" size={24} />
        </View>
      </Pressable>
      {playing && <VideoModal attachment={attachment} onClose={() => setPlaying(false)} />}
    </>
  );
}

function VideoModal({ attachment, onClose }: { attachment: media.Attachment; onClose(): void }) {
  const { t } = useTranslation("notes");
  const insets = useSafeAreaInsets();
  const source = attachment.video?.hlsUrl || attachment.video?.mp4Url || attachment.url;
  const player = useVideoPlayer(absolute(source), (created) => created.play());
  return (
    <Modal visible animationType="fade" onRequestClose={onClose}>
      <View className="flex-1 bg-black">
        <VideoView player={player} style={{ flex: 1 }} nativeControls contentFit="contain" />
        <Pressable
          accessibilityRole="button"
          accessibilityLabel={t("media.close")}
          onPress={onClose}
          className="absolute right-4 rounded-full bg-black/60 p-2"
          style={{ top: insets.top + 8 }}
        >
          <X color="#ffffff" size={22} />
        </Pressable>
      </View>
    </Modal>
  );
}
