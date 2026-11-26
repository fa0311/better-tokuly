import { notes } from "@tokuly/api";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { View } from "react-native";
import { Tabs } from "#/components/tabs";
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
      <View className="flex-row px-4 py-2">
        <Tabs
          testID="user-notes-tab"
          items={(Object.keys(FILTERS) as Tab[]).map((key) => ({
            value: key,
            label: t(`profile.tabs.${key}`),
          }))}
          value={tab}
          onChange={setTab}
        />
      </View>
      <Timeline
        key={tab}
        source={{ kind: "user", userId, filter: FILTERS[tab] }}
        empty={{ emoji: "📝", title: t(`profile.empty.${tab}.${who}`) }}
      />
    </View>
  );
}
