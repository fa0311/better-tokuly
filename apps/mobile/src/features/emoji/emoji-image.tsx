import { type CatalogEmoji, stillUrl } from "@tokuly/core/emoji";
import { Image } from "expo-image";
import { PixelRatio } from "react-native";
import { mediaUrl } from "#/lib/config";

/**
 * One custom emoji at `size` points: animated ones move, still ones load the
 * smallest rendition that is sharp on this screen. Sensitive ones are
 * blurred unless `reveal`.
 */
export function EmojiImage({
  entry,
  size,
  reveal,
}: {
  entry: CatalogEmoji;
  size: number;
  reveal?: boolean;
}) {
  const url = entry.animated ? entry.url : stillUrl(entry, size, PixelRatio.get());
  return (
    <Image
      accessible
      accessibilityRole="image"
      accessibilityLabel={`:${entry.name}:`}
      source={mediaUrl(url)}
      contentFit="contain"
      blurRadius={entry.sensitive && !reveal ? 12 : 0}
      style={{ width: size, height: size }}
    />
  );
}
