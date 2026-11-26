import type { common } from "@tokuly/api";
import { Image } from "expo-image";
import { Text, View } from "react-native";
import { SERVER_URL } from "#/lib/config";
import { sticker, useTheme } from "#/lib/theme";

const PIXELS = { xs: 24, sm: 32, md: 44, lg: 64, xl: 96 } as const;
export type AvatarSize = keyof typeof PIXELS;

/** Fallback tile colours, picked from the user ID so a person keeps theirs (as on the web). */
const TINTS = [
  { bg: "bg-brand", text: "text-on-brand" },
  { bg: "bg-lime", text: "text-[#17122b]" },
  { bg: "bg-pink", text: "text-white" },
  { bg: "bg-sky", text: "text-[#17122b]" },
  { bg: "bg-sun", text: "text-[#17122b]" },
  { bg: "bg-coral", text: "text-white" },
  { bg: "bg-mint", text: "text-[#17122b]" },
] as const;

function tintFor(seed: string) {
  let hash = 0;
  for (let i = 0; i < seed.length; i++) hash = (hash * 31 + seed.charCodeAt(i)) >>> 0;
  return TINTS[hash % TINTS.length] ?? TINTS[0];
}

/** Smallest stored variant that still covers the avatar on a 3x screen. */
function pickUrl(image: common.Image | undefined, pixels: number): string | undefined {
  if (!image) return undefined;
  const wanted = pixels * 3;
  const fitting = image.variants.filter((v) => v.size >= wanted).sort((a, b) => a.size - b.size)[0];
  const url = fitting?.url || image.url;
  if (!url) return undefined;
  return url.startsWith("/") ? `${SERVER_URL}${url}` : url;
}

export interface AvatarProps {
  user?: Pick<common.UserSummary, "id" | "displayName" | "handle" | "avatar" | "deleted">;
  size?: AvatarSize;
  /** The ink outline that makes the avatar a "sticker" (map pins, headers). */
  outlined?: boolean;
  /** Green dot: the person has the app open. */
  online?: boolean;
}

/**
 * A person's face: a rounded square, like the original Tokuly map marker.
 * Falls back to their initial on a colour derived from their ID.
 */
export function Avatar({ user, size = "md", outlined, online }: AvatarProps) {
  const { colors } = useTheme();
  const pixels = PIXELS[size];
  const name = user?.displayName || user?.handle || "?";
  const url = user?.deleted ? undefined : pickUrl(user?.avatar, pixels);
  const tint = tintFor(user?.id ?? name);
  const radius = Math.round(pixels * 0.27);
  return (
    <View style={{ width: pixels, height: pixels }}>
      <View
        className={`items-center justify-center overflow-hidden ${
          url ? "bg-surface-2" : user?.deleted ? "bg-surface-3" : tint.bg
        }`}
        style={[
          { width: pixels, height: pixels, borderRadius: radius },
          // Only when there is one: an `undefined` here would still replace the class's colour.
          url && user?.avatar?.dominantColor
            ? { backgroundColor: user.avatar.dominantColor }
            : null,
          outlined ? sticker(colors, "sm") : null,
        ]}
      >
        {url ? (
          <Image source={url} style={{ width: "100%", height: "100%" }} contentFit="cover" />
        ) : (
          <Text
            className={`font-black ${user?.deleted ? "text-ink-faint" : tint.text}`}
            style={{ fontSize: Math.round(pixels * 0.42) }}
          >
            {user?.deleted ? "?" : Array.from(name)[0]?.toUpperCase()}
          </Text>
        )}
      </View>
      {online && (
        <View
          className="absolute -right-0.5 -bottom-0.5 rounded-full border-2 border-surface bg-success"
          style={{ width: Math.max(10, pixels * 0.28), height: Math.max(10, pixels * 0.28) }}
        />
      )}
    </View>
  );
}
