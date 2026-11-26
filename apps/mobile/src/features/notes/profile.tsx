import { notes } from "@tokuly/api";
import { router } from "expo-router";
import { useTranslation } from "react-i18next";
import { Pressable, Text, View } from "react-native";
import { Section } from "#/components/page";
import type { ProfileSection, ProfileSectionProps } from "#/seams/profile";
import { useDeletedNotes, useTimeline } from "./data";
import { NoteCard } from "./note-card";

/** How many of a person's notes the profile shows before "see all". */
const SHOWN = 3;

/** The newest of a person's notes (pinned ones first), and the way to all of them. */
function ProfileNotes({ user, isSelf }: ProfileSectionProps) {
  const { t } = useTranslation("notes");
  const { t: tm } = useTranslation("mobile");
  const timeline = useTimeline({
    kind: "user",
    userId: user.id,
    filter: notes.UserTimelineFilter.NOTES,
  });
  const deleted = useDeletedNotes();
  const first = timeline.data?.pages[0];
  const pinned = (first?.pinnedNotes ?? []).filter((note) => !deleted.has(note.id));
  const latest = (first?.entries ?? []).filter(
    (entry) => entry.note && !deleted.has(entry.note.id),
  );
  const shown = [
    ...pinned.map((note) => ({ key: `pinned:${note.id}`, note, pinned: true, renote: undefined })),
    ...latest.map((entry) => ({
      key: entry.note!.id,
      note: entry.note!,
      pinned: false,
      renote: entry.renote,
    })),
  ].slice(0, SHOWN);
  if (timeline.isPending || timeline.isError) return null;
  return (
    <Section title={t("profile.title")}>
      {shown.length === 0 ? (
        <Text className="px-4 py-4 text-center text-sm text-ink-soft">
          {t(`profile.empty.notes.${isSelf ? "self" : "other"}`)}
        </Text>
      ) : (
        <View testID="profile-notes">
          {shown.map((row) => (
            <NoteCard key={row.key} note={row.note} pinned={row.pinned} renote={row.renote} />
          ))}
        </View>
      )}
      <Pressable
        testID="profile-notes-all"
        accessibilityRole="button"
        onPress={() => router.push(`/notes/user/${user.id}`)}
        className="px-4 py-3 active:bg-surface-2"
      >
        <Text className="text-center font-black text-brand">{tm("notes.seeAll")}</Text>
      </Pressable>
    </Section>
  );
}

export default [
  { id: "notes", slot: "content", order: 30, Component: ProfileNotes },
] satisfies ProfileSection[];
