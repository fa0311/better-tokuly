import { FlashList, type FlashListRef } from "@shopify/flash-list";
import { useQueryClient } from "@tanstack/react-query";
import type { notes } from "@tokuly/api";
import {
  countNewEntries,
  entryKey,
  notesKeys,
  type TimelineData,
  type TimelinePage,
  type TimelineSource,
  withFreshEntries,
} from "@tokuly/core/notes";
import { type ReactElement, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Alert, Pressable, Text, View } from "react-native";
import { Button } from "#/components/button";
import { EmptyState, Spinner } from "#/components/page";
import { useMe } from "#/features/session/store";
import { errorMessage } from "#/lib/errors";
import { freshPage, useDeletedNotes, useTimeline } from "./data";
import { onTimelineHint } from "./live";
import { NoteCard } from "./note-card";
import { shouldScrollToNewOwnEntry } from "./timeline-scroll";
import { reportViewed, VIEWABILITY } from "./views";

/** A timeline row whose note is there (a row without one has nothing to show). */
type Entry = notes.TimelineEntry & { note: notes.Note };

/** Hints closer together than this lead to one look, not several. */
const LOOK_GAP_MS = 2500;

type Row = { kind: "pinned"; note: notes.Note } | { kind: "entry"; entry: Entry };

const rowKey = (row: Row) =>
  row.kind === "pinned" ? `pinned:${row.note.id}` : entryKey(row.entry);

export interface TimelineProps {
  source: TimelineSource;
  empty: { emoji: string; title: string; body?: string };
  /** Above the first note (a hashtag's header, a profile's tabs). */
  header?: ReactElement;
  testID?: string;
}

/**
 * A timeline, newest first, paged as it scrolls. It never reads its pages
 * again by itself: when something new may be there (a hint from realtime,
 * the app coming back) the newest page is read and a pill says how many
 * notes wait on top; a pull, or the pill, puts them there.
 */
export function Timeline({ source, empty, header, testID }: TimelineProps) {
  const { t } = useTranslation("notes");
  const queryClient = useQueryClient();
  const timeline = useTimeline(source);
  const deleted = useDeletedNotes();
  const myId = useMe()?.user?.id ?? "";
  const list = useRef<FlashListRef<Row>>(null);
  const [waiting, setWaiting] = useState<{ page: TimelinePage; count?: number } | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const key = notesKeys.timeline(source);

  // Looks for new notes without moving what is on screen. One look at a time, and none before the timeline is there.
  const looking = useRef(false);
  const lastLook = useRef(0);
  const look = async () => {
    if (looking.current || !queryClient.getQueryData(key)) return;
    looking.current = true;
    try {
      const page = await freshPage(source);
      lastLook.current = Date.now();
      const count = countNewEntries(queryClient.getQueryData<TimelineData>(key), page);
      if (count !== 0) setWaiting({ page, count });
    } catch {
      // Nobody asked: the next hint, or a pull, tries again.
    } finally {
      looking.current = false;
    }
  };
  const latestLook = useRef(look);
  useEffect(() => {
    latestLook.current = look;
  });
  // A burst of hints (every count that moves on screen is one) leads to one look.
  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | undefined;
    const off = onTimelineHint(() => {
      clearTimeout(timer);
      timer = setTimeout(
        () => void latestLook.current(),
        Math.max(300, LOOK_GAP_MS - (Date.now() - lastLook.current)),
      );
    });
    return () => {
      off();
      clearTimeout(timer);
    };
  }, []);

  const putOnTop = (page: TimelinePage) => {
    queryClient.setQueryData<TimelineData>(key, (current) => withFreshEntries(current, page));
    setWaiting(null);
    list.current?.scrollToOffset({ offset: 0, animated: true });
  };
  const refresh = async () => {
    setRefreshing(true);
    try {
      putOnTop(await freshPage(source));
    } catch (error) {
      Alert.alert(errorMessage(error, { namespace: "notes" }));
    } finally {
      setRefreshing(false);
    }
  };

  // A note shows once: not again under its pinned copy, nor when a later page repeats it (notes
  // posted meanwhile push rows down a page).
  const pages = timeline.data?.pages ?? [];
  const pinned = (pages[0]?.pinnedNotes ?? []).filter((note) => !deleted.has(note.id));
  const shown = new Set(pinned.map((note) => note.id));
  const entries = pages
    .flatMap((page) => page.entries)
    .filter((entry): entry is Entry => {
      const id = entry.note?.id;
      if (!id || shown.has(id) || deleted.has(id)) return false;
      shown.add(id);
      return true;
    });
  const rows: Row[] = [
    ...pinned.map((note): Row => ({ kind: "pinned", note })),
    ...entries.map((entry): Row => ({ kind: "entry", entry })),
  ];
  const newestEntry = entries[0];
  const newestEntryKey = newestEntry ? entryKey(newestEntry) : undefined;
  const previousNewestEntry = useRef<string | undefined>(undefined);

  // A local post is prepended while this screen is behind the composer. FlashList preserves
  // the old visible row when data is inserted before it, which can leave that post offscreen.
  // Put the viewer's own new top entry in view when the composer returns.
  useEffect(() => {
    if (
      shouldScrollToNewOwnEntry(
        previousNewestEntry.current,
        newestEntryKey,
        newestEntry?.note.author?.id,
        myId,
      )
    ) {
      list.current?.scrollToOffset({ offset: 0, animated: false });
    }
    previousNewestEntry.current = newestEntryKey;
  }, [newestEntryKey, newestEntry?.note.author?.id, myId]);

  if (timeline.isPending) return <Spinner />;
  if (timeline.isError)
    return (
      <View className="items-center gap-3 py-10">
        <Text className="px-6 text-center text-ink-soft">
          {errorMessage(timeline.error, { namespace: "notes" })}
        </Text>
        <Button variant="secondary" size="sm" onPress={() => void timeline.refetch()}>
          {t("timeline.retry")}
        </Button>
      </View>
    );
  return (
    <View className="flex-1" testID={testID}>
      <FlashList
        ref={list}
        data={rows}
        keyExtractor={rowKey}
        renderItem={({ item }) =>
          item.kind === "pinned" ? (
            <NoteCard note={item.note} pinned />
          ) : (
            <NoteCard note={item.entry.note} renote={item.entry.renote} />
          )
        }
        ListHeaderComponent={header}
        ListEmptyComponent={<EmptyState {...empty} />}
        ListFooterComponent={timeline.isFetchingNextPage ? <Spinner /> : null}
        onEndReached={() => {
          if (timeline.hasNextPage && !timeline.isFetchingNextPage) void timeline.fetchNextPage();
        }}
        onRefresh={() => void refresh()}
        refreshing={refreshing}
        viewabilityConfig={VIEWABILITY}
        onViewableItemsChanged={({ changed }) =>
          reportViewed(
            changed.flatMap((token) =>
              token.isViewable && token.item
                ? [token.item.kind === "pinned" ? token.item.note : token.item.entry.note]
                : [],
            ),
            myId,
          )
        }
      />
      {waiting && (
        <Pressable
          testID="timeline-new"
          accessibilityRole="button"
          onPress={() => putOnTop(waiting.page)}
          className="absolute top-3 self-center rounded-full bg-brand px-4 py-2"
        >
          <Text className="font-black text-on-brand">
            {waiting.count === undefined
              ? t("timeline.newMany")
              : t("timeline.new", { count: waiting.count })}
          </Text>
        </Pressable>
      )}
    </View>
  );
}
