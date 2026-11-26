import { FlashList } from "@shopify/flash-list";
import type { notes } from "@tokuly/api";
import type { ReactElement } from "react";
import { EmptyState, Spinner } from "#/components/page";
import { useMe } from "#/features/session/store";
import { useDeletedNotes } from "./data";
import { NoteCard } from "./note-card";
import { reportViewed, VIEWABILITY } from "./views";

/** Notes that are not a timeline (a clip's): one card each, paged as it scrolls. */
export function NoteList({
  notes: list,
  pending,
  empty,
  header,
  onEndReached,
  onRefresh,
  refreshing,
}: {
  notes: notes.Note[];
  pending: boolean;
  empty: { emoji: string; title: string; body?: string };
  header?: ReactElement;
  onEndReached(): void;
  onRefresh(): void;
  refreshing: boolean;
}) {
  const deleted = useDeletedNotes();
  const myId = useMe()?.user?.id ?? "";
  if (pending) return <Spinner />;
  return (
    <FlashList
      data={list.filter((note) => !deleted.has(note.id))}
      keyExtractor={(note) => note.id}
      renderItem={({ item }) => <NoteCard note={item} />}
      ListHeaderComponent={header}
      ListEmptyComponent={<EmptyState {...empty} />}
      onEndReached={onEndReached}
      onRefresh={onRefresh}
      refreshing={refreshing}
      viewabilityConfig={VIEWABILITY}
      onViewableItemsChanged={({ changed }) =>
        reportViewed(
          changed.flatMap((token) => (token.isViewable && token.item ? [token.item] : [])),
          myId,
        )
      }
    />
  );
}
