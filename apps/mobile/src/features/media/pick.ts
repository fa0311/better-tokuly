import * as ImagePicker from "expo-image-picker";
import VideoTrim, { type FinishTrimmingEvent, showEditor } from "react-native-video-trim";
import i18n from "#/lib/i18n";
import type { LocalMedia } from "./posts";

function fromAsset(asset: ImagePicker.ImagePickerAsset): LocalMedia {
  const kind = asset.type === "video" ? "video" : "image";
  const contentType = asset.mimeType ?? (kind === "video" ? "video/mp4" : "image/jpeg");
  const extension = contentType.split("/")[1]?.replace("quicktime", "mov") ?? "bin";
  return {
    uri: asset.uri,
    kind,
    name: asset.fileName ?? `${kind}.${extension}`,
    contentType,
    // The picker gives milliseconds.
    duration: asset.duration ? asset.duration / 1000 : undefined,
  };
}

export type PickResult = { media: LocalMedia[] } | { denied: true } | null;

/**
 * Photos and videos from the library. iPhone photos (HEIC) come as JPEG:
 * the server decodes those and strips what they carry. `null` when nothing
 * was picked.
 */
export async function pickFromLibrary(options: {
  kinds: ("images" | "videos")[];
  limit: number;
}): Promise<PickResult> {
  const permission = await ImagePicker.requestMediaLibraryPermissionsAsync();
  if (!permission.granted) return { denied: true };
  const result = await ImagePicker.launchImageLibraryAsync({
    mediaTypes: options.kinds,
    allowsMultipleSelection: options.limit > 1,
    selectionLimit: options.limit,
    quality: 0.9,
    exif: false,
    preferredAssetRepresentationMode:
      ImagePicker.UIImagePickerPreferredAssetRepresentationMode.Compatible,
  });
  if (result.canceled) return null;
  return { media: result.assets.slice(0, options.limit).map(fromAsset) };
}

/**
 * Lets the person cut a video down to `maxSeconds` in the system-style
 * trimmer. Resolves to the trimmed copy, or `null` when they cancelled.
 */
export function trimVideo(video: LocalMedia, maxSeconds: number): Promise<LocalMedia | null> {
  return new Promise((resolve) => {
    const subscriptions = [
      VideoTrim.onFinishTrimming(({ outputPath, duration }: FinishTrimmingEvent) => {
        done();
        resolve({
          ...video,
          uri: outputPath.startsWith("file://") ? outputPath : `file://${outputPath}`,
          name: video.name.replace(/\.[^.]+$/, ".mp4"),
          contentType: "video/mp4",
          duration: duration / 1000,
        });
      }),
      VideoTrim.onCancel(() => {
        done();
        resolve(null);
      }),
      VideoTrim.onError(() => {
        done();
        resolve(null);
      }),
    ];
    const done = () => {
      for (const subscription of subscriptions) subscription.remove();
    };
    showEditor(video.uri, {
      maxDuration: maxSeconds * 1000,
      type: "video",
      saveToPhoto: false,
      autoplay: true,
      cancelButtonText: i18n.t("mobile:media.trimCancel"),
      saveButtonText: i18n.t("mobile:media.trimDone"),
      enableCancelDialog: false,
      enableSaveDialog: false,
    });
  });
}
