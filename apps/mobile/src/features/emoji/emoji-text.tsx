import { splitEmojiText, stillUrl } from "@tokuly/core/emoji";
import type { ReactNode } from "react";
import { Image, PixelRatio } from "react-native";
import { mediaUrl } from "#/lib/config";
import { useCatalog } from "./catalog";

/**
 * Text with the `:shortcode:`s of the catalogue drawn as their emoji, for
 * inside a `<Text>`: what is not an emoji stays as typed. `size` follows the
 * text around it and picks the still that is sharp on this screen. A
 * sensitive emoji stays as its code: a picture inside text cannot be
 * blurred on every platform.
 */
export function useEmojiText(): (text: string, size: number) => ReactNode[] {
  const catalog = useCatalog();
  const drawable = (name: string) => catalog.byName.get(name)?.sensitive === false;
  return (text, size) =>
    splitEmojiText(text, drawable).map((segment, index) => {
      if (segment.kind === "text") return segment.text;
      const entry = catalog.byName.get(segment.name)!;
      return (
        <Image
          key={index}
          accessibilityLabel={segment.raw}
          source={{ uri: mediaUrl(stillUrl(entry, size, PixelRatio.get())) }}
          style={{ width: size, height: size }}
        />
      );
    });
}
