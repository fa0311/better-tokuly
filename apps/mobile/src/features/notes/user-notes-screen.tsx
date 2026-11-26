import { notes } from "@tokuly/api";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Pressable, ScrollView, Text, View } from "react-native";
import { useMe } from "#/features/session/store";
import { Timeline } from "./timeline";

const FILTERS = {
  notes: notes.UserTimelineFilter.NOTES,
  replies: notes.UserTimelineFilter.WITH_REPLIES,
  media: notes.UserTimelineFilter.MEDIA,
  likes: notes.UserTimelineFilter.LIKES,
} as const;
type Tab = keyof typeof FILTERS;

/** A person's notes: their notes (pinned first), with replies, with photos or a video, and what they liked. */
export function UserNotesScreen({ userId }: { userId: string }) {
  const { t } = useTranslation("notes");
  const [tab, setTab] = useState<Tab>("notes");
  const who = useMe()?.user?.id === userId ? "self" : "other";
  return (
    <View className="flex-1 bg-bg" testID="user-notes-screen">
      <View>
        <ScrollView
          horizontal
          showsHorizontalScrollIndicator={false}
          contentContainerClassName="gap-2 px-4 py-2"
        >
          {(Object.keys(FILTERS) as Tab[]).map((key) => (
            <Pressable
              key={key}
              testID={`user-notes-tab-${key}`}
              accessibilityRole="tab"
              accessibilityState={{ selected: tab === key }}
              onPress={() => setTab(key)}
              className={`rounded-full px-3 py-2 ${tab === key ? "bg-ink" : "bg-surface"}`}
            >
              <Text className={`font-black ${tab === key ? "text-surface" : "text-ink"}`}>
                {t(`profile.tabs.${key}`)}
              </Text>
            </Pressable>
          ))}
        </ScrollView>
      </View>
      <Timeline
        key={tab}
        source={{ kind: "user", userId, filter: FILTERS[tab] }}
        empty={{ emoji: "📝", title: t(`profile.empty.${tab}.${who}`) }}
      />
    </View>
  );
}
