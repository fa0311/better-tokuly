import { Pressable, ScrollView, Text, View } from "react-native";
import { CountBadge } from "#/components/page";
import { sticker, useTheme } from "#/lib/theme";

export interface TabItem<T extends string> {
  value: T;
  label: string;
  /** A small count after the label: what waits in that view. */
  badge?: number;
}

/**
 * A pill-shaped segmented control for the views of one screen (the web's
 * `Tabs`): the chosen one is a small sticker on the rail, which scrolls
 * sideways when the labels do not fit. A tab's test ID is `<testID>-<value>`.
 */
export function Tabs<T extends string>({
  items,
  value,
  onChange,
  testID,
}: {
  items: TabItem<NoInfer<T>>[];
  value: T;
  onChange(value: NoInfer<T>): void;
  testID: string;
}) {
  const { colors } = useTheme();
  return (
    <View className="shrink overflow-hidden rounded-full bg-surface-3">
      <ScrollView
        horizontal
        showsHorizontalScrollIndicator={false}
        contentContainerClassName="gap-1 p-1"
      >
        {items.map((item) => {
          const active = item.value === value;
          return (
            <Pressable
              key={item.value}
              testID={`${testID}-${item.value}`}
              accessibilityRole="tab"
              accessibilityState={{ selected: active }}
              onPress={() => onChange(item.value)}
              className={`h-9 flex-row items-center gap-1.5 rounded-full px-4 ${active ? "bg-surface" : ""}`}
              style={
                active ? sticker(colors, "sm") : { borderWidth: 2, borderColor: "transparent" }
              }
            >
              <Text
                numberOfLines={1}
                className={`text-sm font-black ${active ? "text-ink" : "text-ink-soft"}`}
              >
                {item.label}
              </Text>
              {item.badge ? <CountBadge count={item.badge} /> : null}
            </Pressable>
          );
        })}
      </ScrollView>
    </View>
  );
}
