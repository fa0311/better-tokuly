import { FlashList } from "@shopify/flash-list";
import type { notes } from "@tokuly/api";
import { router } from "expo-router";
import { useTranslation } from "react-i18next";
import { Pressable, Text, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { EmptyState, Spinner } from "#/components/page";
import { useMe } from "#/features/session/store";
import { composeHref } from "./compose-href";
import { useDeletedNotes, useThread } from "./data";
import { NoteCard } from "./note-card";
import { reportViewed, VIEWABILITY } from "./views";

type Row =
  | { kind: "ancestor"; note: notes.Note }
  | { kind: "focus"; note: notes.Note }
  | { kind: "replies"; count: number }
  | { kind: "reply"; note: notes.Note };

const rowKey = (row: Row) => (row.kind === "replies" ? "replies" : `${row.kind}:${row.note.id}`);

/**
 * A note in its conversation: what it answers (from the start), the note
 * itself, then its replies oldest first, paged. Writing a reply is the bar
 * at the bottom.
 */
export function NoteScreen({ noteId }: { noteId: string }) {
  const { t } = useTranslation("notes");
  const thread = useThread(noteId);
  const deleted = useDeletedNotes();
  const myId = useMe()?.user?.id ?? "";
  const first = thread.data?.pages[0];
  const note = first?.note;

  if (thread.isPending) return <Spinner />;
  if (!note || deleted.has(note.id))
    return (
      <View className="flex-1 bg-bg" testID="note-not-found">
        <EmptyState emoji="🫥" title={t("thread.notFound")} body={t("thread.notFoundBody")} />
      </View>
    );

  const replies = (thread.data?.pages ?? [])
    .flatMap((page) => page.replies)
    .filter((reply) => !deleted.has(reply.id));
  const rows: Row[] = [
    ...(first?.ancestors ?? [])
      .filter((ancestor) => !deleted.has(ancestor.id))
      .map((ancestor): Row => ({ kind: "ancestor", note: ancestor })),
    { kind: "focus", note },
    { kind: "replies", count: Number(note.counters?.replyCount ?? 0n) },
    ...replies.map((reply): Row => ({ kind: "reply", note: reply })),
  ];
  return (
    <SafeAreaView edges={["bottom"]} className="flex-1 bg-bg" testID="note-screen">
      <FlashList
        data={rows}
        keyExtractor={rowKey}
        renderItem={({ item }) =>
          item.kind === "replies" ? (
            <Text className="px-4 pt-4 pb-2 text-xs font-black text-ink-soft">
              {item.count > 0
                ? t("thread.repliesCount", { count: item.count })
                : t("thread.noReplies")}
            </Text>
          ) : (
            <NoteCard note={item.note} focused={item.kind === "focus"} />
          )
        }
        onEndReached={() => {
          if (thread.hasNextPage && !thread.isFetchingNextPage) void thread.fetchNextPage();
        }}
        onRefresh={() => void thread.refetch()}
        refreshing={thread.isRefetching && !thread.isFetchingNextPage}
        ListFooterComponent={thread.isFetchingNextPage ? <Spinner /> : null}
        viewabilityConfig={VIEWABILITY}
        onViewableItemsChanged={({ changed }) =>
          reportViewed(
            changed.flatMap((token) =>
              token.isViewable && token.item && token.item.kind !== "replies"
                ? [token.item.note]
                : [],
            ),
            myId,
          )
        }
      />
      <Pressable
        testID="thread-reply"
        accessibilityRole="button"
        onPress={() => router.push(composeHref({ replyTo: note.id }))}
        className="mx-4 mb-2 rounded-full bg-surface px-4 py-3"
      >
        <Text className="text-ink-faint">
          {t("thread.replyPlaceholder", { handle: note.author?.handle ?? "" })}
        </Text>
      </Pressable>
    </SafeAreaView>
  );
}
