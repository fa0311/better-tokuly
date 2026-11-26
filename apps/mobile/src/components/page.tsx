import { ChevronRight } from "lucide-react-native";
import type { ReactNode } from "react";
import { ActivityIndicator, Pressable, ScrollView, Text, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { useTheme } from "#/lib/theme";

/** A scrolling screen on the app's paper background. */
export function Page({ children, testID }: { children: ReactNode; testID?: string }) {
  return (
    <SafeAreaView edges={["bottom"]} className="flex-1 bg-bg">
      <ScrollView
        testID={testID}
        contentContainerClassName="gap-4 px-4 pt-4 pb-12"
        keyboardShouldPersistTaps="handled"
      >
        {children}
      </ScrollView>
    </SafeAreaView>
  );
}

/** A heading over a group of rows, and a line on what they are about (`lead`). */
export function Section({
  title,
  lead,
  children,
}: {
  title?: string;
  lead?: string;
  children: ReactNode;
}) {
  return (
    <View className="gap-2">
      {title && (
        <Text className="px-1 text-xs font-black tracking-wider text-ink-soft uppercase">
          {title}
        </Text>
      )}
      {lead && <Text className="-mt-1 px-1 text-xs text-ink-soft">{lead}</Text>}
      <View className="overflow-hidden rounded-card bg-surface">{children}</View>
    </View>
  );
}

/**
 * A tappable row of a `Section`: what it opens, a hint under it, and what
 * waits behind it (`badge`). `danger` rows act instead of opening something.
 */
export function Row({
  icon,
  label,
  hint,
  badge,
  danger,
  onPress,
  testID,
}: {
  icon?: ReactNode;
  label: string;
  hint?: string;
  badge?: number;
  danger?: boolean;
  onPress?(): void;
  testID?: string;
}) {
  const { colors } = useTheme();
  return (
    <Pressable
      testID={testID}
      accessibilityRole="button"
      onPress={onPress}
      className="min-h-14 flex-row items-center gap-3 border-b border-line px-4 py-3 active:bg-surface-2"
    >
      {icon}
      <View className="flex-1">
        <Text className={`text-base font-bold ${danger ? "text-danger" : "text-ink"}`}>
          {label}
        </Text>
        {hint && <Text className="text-xs text-ink-soft">{hint}</Text>}
      </View>
      {badge ? <CountBadge count={badge} /> : null}
      {!danger && <ChevronRight size={18} color={colors["ink-faint"]} />}
    </Pressable>
  );
}

/** A small pink count: what is unread or waits. */
export function CountBadge({ count }: { count: number }) {
  return (
    <View className="min-w-5 items-center rounded-full bg-pink px-1.5">
      <Text className="text-[11px] font-black text-white">{count > 99 ? "99+" : count}</Text>
    </View>
  );
}

/** A friendly placeholder for a list with nothing in it; `action` is the way out of the emptiness. */
export function EmptyState({
  emoji,
  title,
  body,
  action,
}: {
  emoji: string;
  title: string;
  body?: string;
  action?: ReactNode;
}) {
  return (
    <View className="items-center gap-2 px-6 py-14">
      <Text className="text-6xl leading-[72px]">{emoji}</Text>
      <Text className="mt-2 text-center text-lg font-black text-ink">{title}</Text>
      {body && <Text className="max-w-xs text-center text-sm leading-5 text-ink-soft">{body}</Text>}
      {action && <View className="mt-3">{action}</View>}
    </View>
  );
}

export function Spinner() {
  const { colors } = useTheme();
  return (
    <View className="items-center py-8">
      <ActivityIndicator color={colors.brand} />
    </View>
  );
}

/** An error message for the screen reader too. */
export function ErrorNote({ children }: { children: string }) {
  return (
    <Text
      accessibilityRole="alert"
      className="rounded-control bg-danger/10 px-4 py-3 text-sm font-bold text-danger"
    >
      {children}
    </Text>
  );
}
