import { common } from "@tokuly/api";
import { Image } from "expo-image";
import { ImageOff } from "lucide-react-native";
import { useState } from "react";
import { View } from "react-native";
import { mediaUrl } from "#/lib/config";
import { useTheme } from "#/lib/theme";

/**
 * A stamp in a square box, keeping its proportions; its name is what a
 * screen reader says. Animated stamps move (animated WebP); Lottie stamps
 * show their still, as the phone has no Lottie player. A stamp that is gone
 * or fails to load is a dashed frame.
 */
export function StampImage({ stamp, size }: { stamp: common.StampRef; size: number }) {
  const { colors } = useTheme();
  const [failed, setFailed] = useState(false);
  const url =
    stamp.format === common.StampFormat.ANIMATED && stamp.animationUrl
      ? stamp.animationUrl
      : stamp.imageUrl;
  if (failed || !url)
    return (
      <View
        accessible
        accessibilityRole="image"
        accessibilityLabel={stamp.name}
        style={{ width: size, height: size }}
        className="items-center justify-center rounded-2xl border-2 border-dashed border-line"
      >
        <ImageOff size={size / 3} color={colors["ink-faint"]} />
      </View>
    );
  return (
    <Image
      accessible
      accessibilityRole="image"
      accessibilityLabel={stamp.name}
      source={mediaUrl(url)}
      contentFit="contain"
      onError={() => setFailed(true)}
      style={{ width: size, height: size }}
    />
  );
}
