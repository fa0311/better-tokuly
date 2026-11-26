import type { TimelineSource } from "@tokuly/core/notes";
import { router } from "expo-router";
import { ListFilter, PenLine } from "lucide-react-native";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Pressable, ScrollView, Text, View } from "react-native";
import { ActionSheet, type SheetAction } from "#/components/action-sheet";
import { webScreenHref } from "#/features/web/handoff";
import { sticker, useTheme } from "#/lib/theme";
import { composeHref } from "./compose-href";
import { useAntennas, useReadableLists } from "./data";
import { Timeline } from "./timeline";

const TABS = ["home", "following", "global", "mentions"] as const;
type Tab = (typeof TABS)[number];

/** What the tab shows: one of the fixed timelines, or a list or an antenna picked from the sheet. */
type Shown =
  | { kind: Tab }
  | { kind: "list"; listId: string; name: string }
  | { kind: "antenna"; antennaId: string; name: string };

const sourceOf = (shown: Shown): TimelineSource =>
  shown.kind === "list"
    ? { kind: "list", listId: shown.listId }
    : shown.kind === "antenna"
      ? { kind: "antenna", antennaId: shown.antennaId }
      : { kind: shown.kind };

const sourceId = (shown: Shown) =>
  shown.kind === "list"
    ? `list:${shown.listId}`
    : shown.kind === "antenna"
      ? `antenna:${shown.antennaId}`
      : shown.kind;

/**
 * The notes tab: friends' notes (home), the people followed, everyone, and
 * mentions, plus the viewer's lists and antennas to read. Writing is the
 * pen; drafts, clips and what stays on the web (channels, mutes, editing
 * lists and antennas) are in the sheet.
 */
export function NotesScreen() {
  const { t } = useTranslation("notes");
  const { t: tm } = useTranslation("mobile");
  const { colors } = useTheme();
  const [shown, setShown] = useState<Shown>({ kind: "home" });
  const [picking, setPicking] = useState(false);
  const lists = useReadableLists();
  const antennas = useAntennas();

  const readable: SheetAction[] = [
    ...[...(lists.mine.data ?? []), ...(lists.followed.data ?? [])].map(
      (list): SheetAction => ({
        key: `list-${list.id}`,
        label: `📋 ${list.name}`,
        onPress: () => setShown({ kind: "list", listId: list.id, name: list.name }),
      }),
    ),
    ...(antennas.data ?? []).map(
      (antenna): SheetAction => ({
        key: `antenna-${antenna.id}`,
        label: `📡 ${antenna.name}`,
        onPress: () => setShown({ kind: "antenna", antennaId: antenna.id, name: antenna.name }),
      }),
    ),
  ];
  const more: SheetAction[] = [
    { key: "drafts", label: t("drafts.title"), onPress: () => router.push("/notes/drafts") },
    { key: "clips", label: tm("notes.clips"), onPress: () => router.push("/notes/clips") },
    {
      key: "channels",
      label: tm("notes.channels"),
      onPress: () => router.push(webScreenHref("/notes/channels") as never),
    },
    {
      key: "edit-lists",
      label: tm("notes.editLists"),
      onPress: () => router.push(webScreenHref("/notes/lists") as never),
    },
    {
      key: "mutes",
      label: t("mutes.title"),
      onPress: () => router.push(webScreenHref("/settings/notes") as never),
    },
  ];

  const picked = shown.kind === "list" || shown.kind === "antenna" ? shown : null;
  const empty =
    shown.kind === "list"
      ? {
          emoji: "📋",
          title: t("notelists:list.emptyTimeline"),
          body: t("notelists:list.emptyTimelineBody"),
        }
      : shown.kind === "antenna"
        ? {
            emoji: "📡",
            title: t("notelists:antennas.emptyTimeline"),
            body: t("notelists:antennas.emptyTimelineBody"),
          }
        : {
            emoji: shown.kind === "mentions" ? "📮" : "📝",
            title: t(`empty.${shown.kind}`),
            body: t(`empty.${shown.kind}Body`),
          };
  return (
    <View className="flex-1 bg-bg" testID="notes-screen">
      <View className="flex-row items-center gap-2 py-2 pr-2">
        <ScrollView
          horizontal
          showsHorizontalScrollIndicator={false}
          contentContainerClassName="gap-2 pl-4"
        >
          {TABS.map((tab) => (
            <Pressable
              key={tab}
              testID={`notes-tab-${tab}`}
              accessibilityRole="tab"
              accessibilityState={{ selected: shown.kind === tab }}
              onPress={() => setShown({ kind: tab })}
              className={`rounded-full px-3 py-2 ${shown.kind === tab ? "bg-ink" : "bg-surface"}`}
            >
              <Text className={`font-black ${shown.kind === tab ? "text-surface" : "text-ink"}`}>
                {t(`tabs.${tab}`)}
              </Text>
            </Pressable>
          ))}
          {picked && (
            <View
              className="rounded-full bg-ink px-3 py-2"
              accessibilityRole="tab"
              accessibilityState={{ selected: true }}
            >
              <Text className="font-black text-surface" numberOfLines={1}>
                {picked.kind === "list" ? "📋" : "📡"} {picked.name}
              </Text>
            </View>
          )}
        </ScrollView>
        <Pressable
          testID="notes-sources"
          accessibilityRole="button"
          accessibilityLabel={tm("notes.sources")}
          onPress={() => setPicking(true)}
          className="rounded-full bg-surface p-2"
        >
          <ListFilter color={colors.ink} size={20} />
        </Pressable>
      </View>
      <Timeline
        key={sourceId(shown)}
        source={sourceOf(shown)}
        empty={empty}
        testID="notes-timeline"
      />
      <Pressable
        testID="notes-compose"
        accessibilityRole="button"
        accessibilityLabel={t("compose.open")}
        onPress={() => router.push(composeHref())}
        className="absolute right-5 bottom-5 size-14 items-center justify-center rounded-full bg-brand"
        style={sticker(colors)}
      >
        <PenLine color={colors["on-brand"]} size={24} />
      </Pressable>
      <ActionSheet
        visible={picking}
        title={readable.length > 0 ? tm("notes.sources") : tm("notes.noSources")}
        actions={[...readable, ...more]}
        onClose={() => setPicking(false)}
      />
    </View>
  );
}
