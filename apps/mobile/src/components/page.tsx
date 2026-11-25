import type { ReactNode } from "react";
import { ActivityIndicator, ScrollView, Text, View } from "react-native";
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

/** A heading over a group of rows. */
export function Section({ title, children }: { title?: string; children: ReactNode }) {
  return (
    <View className="gap-2">
      {title && (
        <Text className="px-1 text-xs font-black tracking-wider text-ink-soft uppercase">
          {title}
        </Text>
      )}
      <View className="overflow-hidden rounded-card bg-surface">{children}</View>
    </View>
  );
}

export function EmptyState({
  emoji,
  title,
  body,
}: {
  emoji: string;
  title: string;
  body?: string;
}) {
  return (
    <View className="items-center gap-2 px-6 py-10">
      <Text className="text-4xl">{emoji}</Text>
      <Text className="text-center text-base font-black text-ink">{title}</Text>
      {body && <Text className="text-center text-sm text-ink-soft">{body}</Text>}
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
