import type { common, notes } from "@tokuly/api";
import { compactCount, type Engagement } from "@tokuly/core/engagement";
import { router } from "expo-router";
import { Heart, MessageCircle, MoreHorizontal, Repeat2, SmilePlus } from "lucide-react-native";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Alert, Pressable, Share, Text, View } from "react-native";
import { ActionSheet, type SheetAction } from "#/components/action-sheet";
import { useEngagement } from "#/features/engagement/data";
import { useMe } from "#/features/session/store";
import { webScreenHref } from "#/features/web/handoff";
import { SERVER_URL } from "#/lib/config";
import { useTheme } from "#/lib/theme";
import {
  noteSubject,
  useDeleteNote,
  useHideRenotes,
  useLike,
  useMuteThread,
  usePin,
  useReact,
  useRenote,
} from "./actions";
import { ClipSheet } from "./clip-sheet";
import { composeHref } from "./compose-href";

/** The reactions offered in one tap; the web's picker has every emoji. */
const QUICK_REACTIONS = ["👍", "❤️", "😂", "🎉", "😮", "🥺"];

function Count({ value }: { value: bigint | number | undefined }) {
  const { i18n } = useTranslation();
  const count = Number(value ?? 0);
  if (count <= 0) return null;
  return (
    <Text className="text-sm font-black text-ink-soft">{compactCount(count, i18n.language)}</Text>
  );
}

/**
 * The row under a note: reply, renote (or quote), like, an emoji reaction
 * and the menu. What the viewer may do is the server's call (`viewer`):
 * locked accounts and narrow audiences cannot be renoted or quoted.
 */
export function NoteActions({ note, renoter }: { note: notes.Note; renoter?: common.UserSummary }) {
  const { t } = useTranslation("notes");
  const { colors } = useTheme();
  const like = useLike();
  const renote = useRenote();
  const [sheet, setSheet] = useState<"renote" | "react" | "more" | "clip">();
  const liked = note.viewer?.liked ?? false;
  const renoted = note.viewer?.renoted ?? false;
  const canRenote = note.viewer?.canRenote ?? false;
  const canQuote = note.viewer?.canQuote ?? false;

  const renoteActions: SheetAction[] = [];
  if (canRenote || renoted)
    renoteActions.push({
      key: "renote",
      label: renoted ? t("actions.undoRenote") : t("actions.renoteNow"),
      onPress: () => void renote(note),
    });
  if (canQuote)
    renoteActions.push({
      key: "quote",
      label: t("actions.quote"),
      onPress: () => router.push(composeHref({ quote: note.id })),
    });

  return (
    <View className="-ml-2 flex-row items-center justify-between pr-2">
      <Pressable
        testID="note-reply"
        accessibilityRole="button"
        accessibilityLabel={t("actions.reply")}
        onPress={() => router.push(composeHref({ replyTo: note.id }))}
        className="h-9 flex-row items-center gap-1 rounded-full px-2"
      >
        <MessageCircle color={colors["ink-soft"]} size={19} />
        <Count value={note.counters?.replyCount} />
      </Pressable>
      <Pressable
        testID="note-renote"
        accessibilityRole="button"
        accessibilityLabel={t("actions.renote")}
        accessibilityState={{ selected: renoted, disabled: renoteActions.length === 0 }}
        onPress={() =>
          renoteActions.length > 0 ? setSheet("renote") : Alert.alert(t("actions.renotePublicOnly"))
        }
        className="h-9 flex-row items-center gap-1 rounded-full px-2"
        style={{ opacity: renoteActions.length > 0 ? 1 : 0.4 }}
      >
        <Repeat2
          color={renoted ? colors.success : colors["ink-soft"]}
          size={20}
          strokeWidth={renoted ? 3 : 2}
        />
        <Count value={note.counters?.renoteCount} />
      </Pressable>
      <Pressable
        testID="note-like"
        accessibilityRole="button"
        accessibilityLabel={t("actions.like")}
        accessibilityState={{ selected: liked }}
        onPress={() => void like(note)}
        className="h-9 flex-row items-center gap-1 rounded-full px-2"
      >
        <Heart
          color={liked ? colors.pink : colors["ink-soft"]}
          fill={liked ? colors.pink : "transparent"}
          size={19}
        />
        <Count value={note.counters?.likeCount} />
      </Pressable>
      <Pressable
        testID="note-react"
        accessibilityRole="button"
        accessibilityLabel={t("mobile:notes.react")}
        onPress={() => setSheet("react")}
        className="h-9 flex-row items-center rounded-full px-2"
      >
        <SmilePlus color={colors["ink-soft"]} size={19} />
      </Pressable>
      <Pressable
        testID="note-more"
        accessibilityRole="button"
        accessibilityLabel={t("menu.label")}
        onPress={() => setSheet("more")}
        className="h-9 flex-row items-center rounded-full px-2"
      >
        <MoreHorizontal color={colors["ink-soft"]} size={19} />
      </Pressable>

      <ActionSheet
        visible={sheet === "renote"}
        actions={renoteActions}
        onClose={() => setSheet(undefined)}
      />
      {sheet === "react" && <ReactionSheet noteId={note.id} onClose={() => setSheet(undefined)} />}
      {sheet === "more" && (
        <MoreSheet
          note={note}
          renoter={renoter}
          onClip={() => setSheet("clip")}
          onClose={() => setSheet(undefined)}
        />
      )}
      {sheet === "clip" && <ClipSheet noteId={note.id} onClose={() => setSheet(undefined)} />}
    </View>
  );
}

/** A row of emoji to react with; the ones the viewer gave are lit and taken back with a tap. */
function ReactionSheet({ noteId, onClose }: { noteId: string; onClose(): void }) {
  const { t } = useTranslation("mobile");
  const engagement = useEngagement(noteSubject(noteId));
  const react = useReact(noteId);
  const given = (emoji: string, data: Engagement | null | undefined) =>
    data?.viewerReactions.some((reaction) => reaction.emoji === emoji) ?? false;
  const counts = new Map(
    (engagement.data?.emojiCounts ?? []).map((count) => [count.emoji, count.count]),
  );
  return (
    <ActionSheet visible title={t("notes.react")} actions={[]} onClose={onClose}>
      <View className="flex-row flex-wrap justify-center gap-2 px-2 pb-3">
        {QUICK_REACTIONS.map((emoji) => {
          const on = given(emoji, engagement.data);
          return (
            <Pressable
              key={emoji}
              testID={`react-${emoji}`}
              accessibilityRole="button"
              accessibilityState={{ selected: on }}
              onPress={() => {
                onClose();
                void react(emoji, !on);
              }}
              className={`min-w-14 items-center rounded-2xl px-3 py-2 ${on ? "bg-brand-soft" : "bg-surface-2"}`}
            >
              <Text className="text-2xl">{emoji}</Text>
              {(counts.get(emoji) ?? 0) > 0 && (
                <Text className="text-xs font-black text-ink-soft">{counts.get(emoji)}</Text>
              )}
            </Pressable>
          );
        })}
      </View>
    </ActionSheet>
  );
}

/**
 * The note's menu: share, clip, mute the conversation, hide someone's
 * renotes and — on the viewer's own notes — pin and delete. Editing, who
 * may see it, the numbers and reporting are on the web screen of the note.
 */
function MoreSheet({
  note,
  renoter,
  onClip,
  onClose,
}: {
  note: notes.Note;
  renoter?: common.UserSummary;
  onClip(): void;
  onClose(): void;
}) {
  const { t } = useTranslation("notes");
  const me = useMe();
  const pin = usePin();
  const hideRenotes = useHideRenotes();
  const remove = useDeleteNote();
  const muteThread = useMuteThread();
  const mine = Boolean(me?.user && note.author?.id === me.user.id);
  const onWeb = () => router.push(webScreenHref(`/notes/${note.id}`) as never);

  const actions: SheetAction[] = [
    {
      key: "share",
      label: t("mobile:notes.share"),
      onPress: () =>
        void Share.share({
          url: `${SERVER_URL}/notes/${note.id}`,
          message: `${SERVER_URL}/notes/${note.id}`,
        }),
    },
    { key: "clip", label: t("clips:menu"), onPress: onClip },
    {
      key: "mute",
      label: note.viewer?.threadMuted ? t("mutes.unmuteThread") : t("mutes.muteThread"),
      onPress: () => void muteThread(note, !note.viewer?.threadMuted),
    },
  ];
  if (renoter && renoter.id !== me?.user?.id)
    actions.push({
      key: "hide-renotes",
      label: t("mutes.hideRenotes", { name: renoter.displayName || renoter.handle }),
      onPress: () => void hideRenotes(renoter.id),
    });
  if (mine) {
    actions.push(
      note.pinned
        ? { key: "unpin", label: t("menu.unpin"), onPress: () => void pin(note, false) }
        : { key: "pin", label: t("menu.pin"), onPress: () => void pin(note, true) },
    );
    if (note.viewer?.canEdit)
      actions.push({ key: "edit", label: t("mobile:notes.editOnWeb"), onPress: onWeb });
    actions.push({
      key: "delete",
      label: t("menu.delete"),
      destructive: true,
      onPress: () =>
        Alert.alert(t("delete.title"), t("delete.body"), [
          { text: t("visibility.cancel"), style: "cancel" },
          {
            text: t("delete.confirm"),
            style: "destructive",
            onPress: () => void remove(note),
          },
        ]),
    });
  } else {
    actions.push({
      key: "report",
      label: t("mobile:notes.reportOnWeb"),
      destructive: true,
      onPress: onWeb,
    });
  }
  return <ActionSheet visible title={t("menu.label")} actions={actions} onClose={onClose} />;
}
