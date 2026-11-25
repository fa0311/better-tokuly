import type { ReactNode } from "react";
import { ActivityIndicator, Pressable, Text, View } from "react-native";
import { sticker, useTheme } from "#/lib/theme";

const VARIANTS = {
  primary: { box: "bg-brand", text: "text-on-brand", outlined: true },
  secondary: { box: "bg-surface", text: "text-ink", outlined: true },
  lime: { box: "bg-lime", text: "text-[#17122b]", outlined: true },
  ghost: { box: "bg-transparent", text: "text-ink-soft", outlined: false },
  danger: { box: "bg-danger", text: "text-white", outlined: true },
} as const;

export interface ButtonProps {
  children: ReactNode;
  onPress?: () => void;
  variant?: keyof typeof VARIANTS;
  size?: "sm" | "md" | "lg";
  icon?: ReactNode;
  loading?: boolean;
  disabled?: boolean;
  block?: boolean;
  testID?: string;
  accessibilityLabel?: string;
}

const HEIGHTS = { sm: 36, md: 44, lg: 52 } as const;

/** A pressable pill with the "sticker" outline; it sinks into its shadow while pressed. */
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
  const look = VARIANTS[variant];
  const inactive = disabled || loading;
  return (
    <Pressable
      testID={testID}
      accessibilityRole="button"
      accessibilityLabel={accessibilityLabel}
      accessibilityState={{ disabled: Boolean(inactive), busy: Boolean(loading) }}
      disabled={inactive}
      onPress={onPress}
      style={({ pressed }) => [
        {
          height: HEIGHTS[size],
          borderRadius: 999,
          paddingHorizontal: size === "sm" ? 14 : 20,
          alignSelf: block ? "stretch" : "flex-start",
          opacity: inactive && !loading ? 0.5 : 1,
          transform: [{ translateY: pressed && look.outlined ? 2 : 0 }],
        },
        look.outlined ? sticker(colors, pressed ? "sm" : "md") : null,
      ]}
      className={`flex-row items-center justify-center gap-2 ${look.box}`}
    >
      {loading ? (
        <ActivityIndicator color={variant === "primary" ? colors["on-brand"] : colors.ink} />
      ) : (
        <View className="flex-row items-center gap-2">
          {icon}
          {typeof children === "string" ? (
            <Text className={`font-black ${look.text} ${size === "sm" ? "text-sm" : "text-base"}`}>
              {children}
            </Text>
          ) : (
            children
          )}
        </View>
      )}
    </Pressable>
  );
}
