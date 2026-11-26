import { common, type media, type notes, toMillis } from "@tokuly/api";
import { compactAge } from "@tokuly/core/format";
import { visibilityOption } from "@tokuly/core/notes";
import { Image } from "expo-image";
import { router } from "expo-router";
import { AtSign, Lock, Pin, Repeat2, Star, UserRoundCheck, UsersRound } from "lucide-react-native";
import { memo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Pressable, Text, View } from "react-native";
import { Avatar } from "#/components/avatar";
import { SERVER_URL } from "#/lib/config";
import i18n from "#/lib/i18n";
import { useTheme } from "#/lib/theme";
import { useNoteTopic } from "./live";
import { NoteMedia } from "./media";
import { NoteActions } from "./note-actions";
import { NotePoll } from "./poll";
import { NoteRichText, openLink } from "./rich-text";

const WEEK_MS = 7 * 24 * 60 * 60 * 1000;

/** "3分前" for a week, then the date (with the year once it is not this one). */
export function noteAge(postedMs: number, nowMs = Date.now()): string {
  if (nowMs - postedMs < WEEK_MS) return compactAge(i18n, postedMs, nowMs);
  const posted = new Date(postedMs);
  const sameYear = posted.getFullYear() === new Date(nowMs).getFullYear();
  return posted.toLocaleDateString(
    i18n.language,
    sameYear
      ? { month: "short", day: "numeric" }
      : { year: "numeric", month: "short", day: "numeric" },
  );
}

const openNote = (noteId: string) => router.push(`/notes/${noteId}`);
const openProfile = (handle: string) => router.push(`/u/${handle}`);

/** A small mark on notes that are not shared with everyone. */
function AudienceMark({ note }: { note: notes.Note }) {
  const { t } = useTranslation("notes");
  const { colors } = useTheme();
  if (
    note.visibility === common.Visibility.PUBLIC ||
    note.visibility === common.Visibility.UNSPECIFIED
  )
    return null;
  const shown = visibilityOption(note.visibility);
  const key = shown.key === "private" && note.directRecipients.length > 0 ? "direct" : shown.key;
  const Icon =
    key === "closeFriends"
      ? Star
      : key === "friends"
        ? UsersRound
        : key === "followers" || key === "circle"
          ? UserRoundCheck
          : key === "direct"
            ? AtSign
            : Lock;
  return (
    <View accessibilityRole="image" accessibilityLabel={t(`visibility.${key}.label`)}>
      <Icon
        color={key === "closeFriends" ? colors.success : colors["ink-faint"]}
        fill={key === "closeFriends" ? colors.success : "transparent"}
        size={13}
      />
    </View>
  );
}

export interface NoteCardProps {
  note: notes.Note;
  /** The row is there because this person renoted the note. */
  renote?: notes.RenoteInfo;
  /** Pinned to the author's profile, shown first there. */
  pinned?: boolean;
  /** The note a thread is about: larger, not a link to itself. */
  focused?: boolean;
}

/**
 * One note: who wrote it and when, what it answers, its text (folded behind
 * a content warning when it has one), photos or a video, a poll, the note
 * it quotes, a link preview, and the row of actions. A tap opens its thread.
 */
export const NoteCard = memo(function NoteCard({ note, renote, pinned, focused }: NoteCardProps) {
  const { t } = useTranslation("notes");
  const { colors } = useTheme();
  useNoteTopic(note.id);
  const author = note.author;
  const posted = toMillis(note.createdAt);
  return (
    <Pressable
      testID={`note-${note.id}`}
      // The card is a group: its name, links and buttons stay reachable one by one.
      accessible={false}
      disabled={focused}
      onPress={() => openNote(note.id)}
      className={`gap-2 border-b border-line bg-surface px-4 py-3 ${focused ? "" : "active:bg-surface-2"}`}
    >
      {renote?.user && (
        <View className="ml-12 flex-row items-center gap-1.5">
          <Repeat2 color={colors.success} size={14} />
          <Text className="text-xs font-bold text-ink-soft" numberOfLines={1}>
            {t("card.renotedBy", { name: renote.user.displayName || renote.user.handle })}
          </Text>
        </View>
      )}
      {pinned && (
        <View className="ml-12 flex-row items-center gap-1.5">
          <Pin color={colors["ink-faint"]} size={13} />
          <Text className="text-xs font-bold text-ink-soft">{t("card.pinned")}</Text>
        </View>
      )}
      <View className="flex-row gap-3">
        <Pressable
          accessibilityRole="link"
          onPress={() => author && openProfile(author.handle)}
          hitSlop={4}
        >
          <Avatar user={author} size="md" />
        </Pressable>
        <View className="min-w-0 flex-1 gap-1.5">
          <View className="flex-row items-center gap-1.5">
            <Text className="shrink font-black text-ink" numberOfLines={1}>
              {author?.displayName || author?.handle}
            </Text>
            <Text className="shrink text-sm text-ink-faint" numberOfLines={1}>
              @{author?.handle}
            </Text>
            <AudienceMark note={note} />
            <Text className="ml-auto text-xs text-ink-faint">
              {noteAge(posted)}
              {note.editedAt ? ` · ${t("card.edited")}` : ""}
            </Text>
          </View>
          {note.replyTo?.author && (
            <Text className="text-xs text-ink-soft" numberOfLines={1}>
              {t("card.replyingTo")}{" "}
              <Text
                className="font-bold text-brand"
                onPress={() => openProfile(note.replyTo!.author!.handle)}
              >
                @{note.replyTo.author.handle}
              </Text>
            </Text>
          )}
          <NoteBody note={note} />
          <NoteActions note={note} renoter={renote?.user} />
        </View>
      </View>
    </Pressable>
  );
});

/** What a note says, shown behind its content warning until tapped. */
export function NoteBody({ note }: { note: notes.Note }) {
  const { t } = useTranslation("notes");
  const [revealed, setRevealed] = useState(false);
  if (note.contentWarning && !revealed)
    return (
      <View className="gap-2 rounded-2xl bg-surface-2 p-3">
        <Text className="font-bold text-ink">
          ⚠️ {note.contentWarningText || t("card.sensitive")}
        </Text>
        <Pressable
          testID="note-reveal"
          accessibilityRole="button"
          onPress={() => setRevealed(true)}
          className="self-start rounded-full bg-ink px-3 py-1.5"
        >
          <Text className="text-sm font-black text-surface">{t("card.reveal")}</Text>
        </Pressable>
      </View>
    );
  return (
    <View className="gap-2">
      <NoteRichText note={note} />
      <NoteMedia note={note} />
      {note.poll && <NotePoll note={note} poll={note.poll} />}
      {note.quote && <QuoteEmbed quote={note.quote} />}
      {note.linkPreview?.url && !note.quote && <LinkPreview preview={note.linkPreview} />}
      {note.place?.name && (
        <Text className="text-xs font-bold text-ink-soft">📍 {note.place.name}</Text>
      )}
    </View>
  );
}

/** The note a quote note shows underneath its own text; a tap opens it. */
function QuoteEmbed({ quote }: { quote: notes.QuotedNote }) {
  const { t } = useTranslation("notes");
  const note = quote.note;
  if (quote.unavailable || !note)
    return (
      <View className="rounded-2xl border border-line p-3">
        <Text className="text-sm text-ink-faint">{t("card.quoteUnavailable")}</Text>
      </View>
    );
  return (
    <Pressable
      testID="note-quote"
      accessibilityRole="link"
      onPress={() => openNote(note.id)}
      className="gap-1.5 rounded-2xl border border-line p-3 active:bg-surface-2"
    >
      <View className="flex-row items-center gap-2">
        <Avatar user={note.author} size="xs" />
        <Text className="shrink font-black text-ink" numberOfLines={1}>
          {note.author?.displayName || note.author?.handle}
        </Text>
        <Text className="text-xs text-ink-faint">{noteAge(toMillis(note.createdAt))}</Text>
      </View>
      {note.contentWarning ? (
        <Text className="text-sm text-ink-soft">
          ⚠️ {note.contentWarningText || t("card.sensitive")}
        </Text>
      ) : (
        <NoteRichText note={note} numberOfLines={4} />
      )}
    </Pressable>
  );
}

const absolute = (url: string) => (url.startsWith("/") ? `${SERVER_URL}${url}` : url);

function LinkPreview({ preview }: { preview: media.LinkPreview }) {
  const image = preview.imageUrl ? absolute(preview.imageUrl) : undefined;
  return (
    <Pressable
      accessibilityRole="link"
      onPress={() => openLink(preview.url)}
      className="flex-row overflow-hidden rounded-2xl border border-line active:bg-surface-2"
    >
      {image && <Image source={image} contentFit="cover" style={{ width: 84, height: 84 }} />}
      <View className="flex-1 justify-center gap-0.5 px-3 py-2">
        <Text className="text-xs text-ink-faint" numberOfLines={1}>
          {preview.siteName || preview.url.replace(/^https?:\/\//, "")}
        </Text>
        {preview.title ? (
          <Text className="font-bold text-ink" numberOfLines={2}>
            {preview.title}
          </Text>
        ) : null}
      </View>
    </Pressable>
  );
}
