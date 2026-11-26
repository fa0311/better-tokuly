import type { ReactNode } from "react";
import { useState } from "react";
import { ActivityIndicator, Pressable, Text } from "react-native";
import { type ColorName, INK_ON_BRIGHT, sticker, useTheme } from "#/lib/theme";

/** The web's button variants: what fills the pill, what is written on it, and whether it is a sticker. */
const VARIANTS = {
  /** The one main action on a screen. */
  primary: { fill: "brand", ink: "on-brand", outlined: true },
  /** Neutral actions next to a primary one. */
  secondary: { fill: "surface", ink: "ink", outlined: true },
  /** Celebratory / positive actions. */
  lime: { fill: "lime", ink: null, outlined: true },
  /** Low-emphasis actions inside dense UI. */
  ghost: { fill: null, ink: "ink", outlined: false },
  danger: { fill: "danger", ink: null, outlined: true },
} satisfies Record<string, { fill: ColorName | null; ink: ColorName | null; outlined: boolean }>;

const FIXED_INK = { lime: INK_ON_BRIGHT, danger: "#ffffff" } as const;

const SIZES = {
  sm: { height: 36, paddingHorizontal: 16, fontSize: 14 },
  md: { height: 44, paddingHorizontal: 20, fontSize: 15 },
  lg: { height: 56, paddingHorizontal: 28, fontSize: 18 },
} as const;

export interface ButtonProps {
  children: ReactNode;
  onPress?: () => void;
  variant?: keyof typeof VARIANTS;
  size?: keyof typeof SIZES;
  icon?: ReactNode;
  loading?: boolean;
  disabled?: boolean;
  block?: boolean;
  testID?: string;
  accessibilityLabel?: string;
}

/**
 * A pressable pill with the "sticker" outline; it sinks into its shadow while pressed.
 *
 * Styled with a plain `style` object: NativeWind drops a `style` function
 * (`({ pressed }) => …`) from a component that also has a `className`, which
 * once left this button a flat rectangle. Pressed is state here instead.
 */
export function Button({
  children,
  onPress,
  variant = "primary",
  size = "md",
  icon,
  loading,
  disabled,
  block,
  testID,
  accessibilityLabel,
}: ButtonProps) {
  const { colors } = useTheme();
  const [pressed, setPressed] = useState(false);
  const look = VARIANTS[variant];
  const { fontSize, ...box } = SIZES[size];
  const ink = look.ink ? colors[look.ink] : FIXED_INK[variant as keyof typeof FIXED_INK];
  const fill = look.fill ? colors[look.fill] : pressed ? colors["surface-2"] : "transparent";
  const inactive = disabled || loading;
  return (
    <Pressable
      testID={testID}
      accessibilityRole="button"
      accessibilityLabel={accessibilityLabel}
      accessibilityState={{ disabled: Boolean(inactive), busy: Boolean(loading) }}
      disabled={inactive}
      onPress={onPress}
      onPressIn={() => setPressed(true)}
      onPressOut={() => setPressed(false)}
      style={[
        box,
        {
          flexDirection: "row",
          alignItems: "center",
          justifyContent: "center",
          gap: 8,
          borderRadius: 999,
          backgroundColor: fill,
          alignSelf: block ? "stretch" : "flex-start",
          opacity: inactive && !loading ? 0.5 : 1,
        },
        look.outlined && sticker(colors, pressed ? "sm" : "md"),
        look.outlined && pressed && { transform: [{ translateY: 2 }] },
      ]}
    >
      {loading ? <ActivityIndicator color={ink} /> : icon}
      {loading ? null : typeof children === "string" || Array.isArray(children) ? (
        <Text numberOfLines={1} style={{ color: ink, fontSize, fontWeight: "900" }}>
          {children}
        </Text>
      ) : (
        children
      )}
    </Pressable>
  );
}
