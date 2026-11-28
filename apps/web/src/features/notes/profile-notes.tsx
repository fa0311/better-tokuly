import { notes, toMillis } from "@tokuly/api";
import { useState } from "react";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { Tabs } from "#/components/ui/tabs";
import { useTranslation } from "#/lib/use-translation";
import { ActivityRow } from "./feed/activity-card";
import { useUserActivity } from "./feed/api";
import { dayLabel, groupByDay } from "./feed/days";
import { ComposeButton, ScrollScope } from "./shell";
import { Timeline } from "./timeline";

type ProfileTab = "notes" | "replies" | "media" | "likes" | "activity";

const FILTERS: Record<Exclude<ProfileTab, "activity">, notes.UserTimelineFilter> = {
  notes: notes.UserTimelineFilter.NOTES,
  replies: notes.UserTimelineFilter.WITH_REPLIES,
  media: notes.UserTimelineFilter.MEDIA,
  likes: notes.UserTimelineFilter.LIKES,
};

export interface ProfileNotesProps {
  userId: string;
  /** The profile is the viewer's own: empty states invite them to post, and the write button is offered. */
  isSelf: boolean;
}

/**
 * The notes part of a profile: the pinned note and the person's notes, with
 * tabs for replies, media, likes and their activity log, as far as the
 * viewer may see them.
 */
export function ProfileNotes({ userId, isSelf }: ProfileNotesProps) {
  const { t } = useTranslation("notes");
  const [tab, setTab] = useState<ProfileTab>("notes");
  const who = isSelf ? "self" : "other";

  return (
    <section className="mt-6" aria-label={t("profile.title")}>
      <Tabs<ProfileTab>
        className="mb-3"
        value={tab}
        onChange={setTab}
        items={[
          { value: "notes", label: t("profile.tabs.notes") },
          { value: "replies", label: t("profile.tabs.replies") },
          { value: "media", label: t("profile.tabs.media") },
          { value: "likes", label: t("profile.tabs.likes") },
          { value: "activity", label: t("profile.tabs.activity") },
        ]}
      />
      {tab === "activity" ? (
        <ProfileActivity userId={userId} />
      ) : (
        <ScrollScope scrollId="profile">
          <Timeline
            key={tab}
            source={{ kind: "user", userId, filter: FILTERS[tab] }}
            empty={
              <EmptyState
                emoji={tab === "likes" ? "💗" : tab === "media" ? "🖼️" : "📝"}
                title={t(`profile.empty.${tab}.${who}`)}
              />
            }
          />
        </ScrollScope>
      )}
      {isSelf && <ComposeButton />}
    </section>
  );
}

/** What the person has been doing, by day, as far as the viewer may see it. */
function ProfileActivity({ userId }: { userId: string }) {
  const { t, i18n } = useTranslation("notes");
  const query = useUserActivity(userId);
  const activities = query.data?.pages.flatMap((page) => page.activities) ?? [];

  if (query.isPending) {
    return (
      <div className="flex flex-col gap-2">
        <Skeleton className="h-16" />
        <Skeleton className="h-16" />
      </div>
    );
  }
  if (activities.length === 0) return <EmptyState emoji="🐾" title={t("profile.empty.activity")} />;
  return (
    <div className="flex flex-col gap-4">
      {groupByDay(activities, (activity) => toMillis(activity.occurredAt)).map((day) => (
        <section key={day.key}>
          <h3 className="mb-1.5 px-1 text-sm font-black text-ink-soft">
            {dayLabel(day.date, i18n.language, t)}
          </h3>
          <div className="flex flex-col gap-1.5">
            {day.items.map((activity) => (
              <ActivityRow key={activity.id} activity={activity} />
            ))}
          </div>
        </section>
      ))}
      <LoadMore query={query} />
    </div>
  );
}
