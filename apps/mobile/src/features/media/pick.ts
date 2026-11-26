import * as ImagePicker from "expo-image-picker";
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
