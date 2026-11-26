import { FlashList } from "@shopify/flash-list";
import { router } from "expo-router";
import { useTranslation } from "react-i18next";
import { Pressable, Text, View } from "react-native";
import { EmptyState, Spinner } from "#/components/page";
import { useClip, useClipNotes, useClips } from "./data";
import { NoteList } from "./note-list";

/** The viewer's clips: notes put aside, in named folders. */
export function ClipsScreen() {
  const { t } = useTranslation("clips");
  const clips = useClips();
  const list = clips.data?.pages.flatMap((page) => page.clips) ?? [];
  if (clips.isPending) return <Spinner />;
  return (
    <View className="flex-1 bg-bg" testID="clips-screen">
      <FlashList
        data={list}
        keyExtractor={(clip) => clip.id}
        renderItem={({ item }) => (
          <Pressable
            testID={`clip-row-${item.id}`}
            accessibilityRole="button"
            onPress={() => router.push(`/notes/clips/${item.id}`)}
            className="gap-0.5 border-b border-line bg-surface px-4 py-3 active:bg-surface-2"
          >
            <View className="flex-row items-center gap-2">
              <Text className="flex-1 font-black text-ink" numberOfLines={1}>
                📎 {item.name}
              </Text>
              <Text className="text-xs text-ink-faint">
                {item.public ? t("public") : t("private")}
              </Text>
            </View>
            {item.description ? (
              <Text className="text-sm text-ink-soft" numberOfLines={2}>
                {item.description}
              </Text>
            ) : null}
            <Text className="text-xs text-ink-faint">
              {t("noteCount", { count: item.noteCount })}
            </Text>
          </Pressable>
        )}
        onEndReached={() => {
          if (clips.hasNextPage && !clips.isFetchingNextPage) void clips.fetchNextPage();
        }}
        onRefresh={() => void clips.refetch()}
        refreshing={clips.isRefetching && !clips.isFetchingNextPage}
        ListEmptyComponent={<EmptyState emoji="📎" title={t("empty")} body={t("emptyBody")} />}
      />
    </View>
  );
}

/** One clip's notes. */
export function ClipScreen({ clipId }: { clipId: string }) {
  const { t } = useTranslation("clips");
  const clip = useClip(clipId);
  const notes = useClipNotes(clipId);
  if (clip.isError || clip.data === null)
    return <EmptyState emoji="📎" title={t("notFound")} body={t("notFoundBody")} />;
  return (
    <View className="flex-1 bg-bg" testID="clip-screen">
      <NoteList
        notes={notes.data?.pages.flatMap((page) => page.notes) ?? []}
        pending={notes.isPending}
        empty={{ emoji: "📎", title: t("notesEmpty"), body: t("notesEmptyBody") }}
        header={
          clip.data ? (
            <View className="gap-1 px-4 py-3">
              <Text className="text-lg font-black text-ink">{clip.data.name}</Text>
              {clip.data.description ? (
                <Text className="text-sm text-ink-soft">{clip.data.description}</Text>
              ) : null}
            </View>
          ) : undefined
        }
        onEndReached={() => {
          if (notes.hasNextPage && !notes.isFetchingNextPage) void notes.fetchNextPage();
        }}
        onRefresh={() => void notes.refetch()}
        refreshing={notes.isRefetching && !notes.isFetchingNextPage}
      />
    </View>
  );
}
