import type { ReactNode } from "react";
import { Pressable } from "react-native";
import { sticker, useTheme } from "#/lib/theme";

const LOOKS = {
  /** A quiet round control inside a bar. */
  plain: { box: "size-10 bg-surface", outlined: false },
  /** A control floating over content (the map). */
  sticker: { box: "size-12 bg-surface", outlined: true },
  /** The screen's main action, floating in the corner. */
  primary: { box: "size-14 bg-brand", outlined: true },
  /** Over video: readable on any frame. */
  glass: { box: "size-10 bg-black/40", outlined: false },
} as const;

/** A round button that is only an icon; `label` is what a screen reader says. */
export function IconButton({
  icon,
  label,
  onPress,
  look = "plain",
  testID,
}: {
  icon: ReactNode;
  label: string;
  onPress(): void;
  look?: keyof typeof LOOKS;
  testID?: string;
}) {
  const { colors } = useTheme();
  const { box, outlined } = LOOKS[look];
  return (
    <Pressable
      testID={testID}
      accessibilityRole="button"
      accessibilityLabel={label}
      onPress={onPress}
      hitSlop={6}
      className={`items-center justify-center rounded-full active:opacity-70 ${box}`}
      style={outlined ? sticker(colors) : undefined}
    >
      {icon}
    </Pressable>
  );
}
