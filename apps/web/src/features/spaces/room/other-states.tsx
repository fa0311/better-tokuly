import { useNavigate } from "@tanstack/react-router";
import { common, spaces, toMillis } from "@tokuly/api";
import { isModerator, minutesOf } from "@tokuly/core/spaces";
import { CalendarClock, UserPlus } from "lucide-react";
import { useState } from "react";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { Card } from "#/components/ui/card";
import { LoadMore } from "#/components/ui/load-more";
import { Page } from "#/components/ui/page";
import { EmojiText } from "#/features/emoji";
import { ReportButton, SubjectMoreButton } from "#/features/engagement";
import { useAccountTimeZone, useAccountToday } from "#/lib/account-time";
import { useTranslation } from "#/lib/use-translation";
import { useMe } from "#/stores/session";
import { useParticipants } from "../api";
import { MySpaceActions } from "../my-space-actions";
import { AudienceLabel, PersonName, TagList } from "../parts";
import { ReminderButton } from "../reminder-button";
import { formatWhen } from "../when";
import { PersonRow } from "./people";
import { InvitesDialog } from "./room-dialogs";

const subjectOf = (space: spaces.Space) => ({ type: common.SubjectType.SPACE, id: space.id });

/** A room that is announced: when it starts, who hosts it, and a reminder (or, for the host, the start button). */
export function ScheduledRoom({ space }: { space: spaces.Space }) {
  const { t, i18n } = useTranslation("spaces");
  const zone = useAccountTimeZone();
  const today = useAccountToday();
  const navigate = useNavigate();
  const [inviting, setInviting] = useState(false);
  const role = space.viewer?.role ?? spaces.SpaceRole.UNSPECIFIED;
  const host = role === spaces.SpaceRole.HOST;
  return (
    <Page
      title={t("scheduled.title")}
      back="/spaces"
      actions={
        <SubjectMoreButton
          subject={subjectOf(space)}
          title={t("share.title", { title: space.title })}
          label={space.title}
          report={!host}
        />
      }
    >
      <Card className="flex flex-col gap-4" data-testid="scheduled-room">
        <p className="inline-flex items-center gap-2 self-start rounded-full bg-sun px-3 py-1 text-sm font-black text-[#17122b]">
          <CalendarClock className="size-4" />
          {formatWhen(toMillis(space.scheduledAt), zone, today, i18n.language)}
        </p>
        <div>
          <AudienceLabel audience={space.audience} className="text-ink-faint" />
          <h2 className="mt-1 text-2xl leading-snug font-black break-words">
            <EmojiText text={space.title} />
          </h2>
          <TagList tags={space.tags} className="mt-2" />
        </div>
        <div className="flex items-center gap-3">
          <Avatar user={space.host} outlined />
          <div className="min-w-0">
            <p className="truncate font-bold">
              <PersonName user={space.host} />
            </p>
            <p className="text-sm text-ink-soft">{t("role.host")}</p>
          </div>
        </div>
        <p className="text-sm font-bold text-ink-soft" data-testid="reminder-count">
          {space.reminderCount > 0
            ? t("scheduled.reminders", { count: space.reminderCount })
            : t("scheduled.noReminders")}
        </p>
        <div className="flex flex-wrap gap-2">
          {host ? (
            <MySpaceActions
              space={space}
              size="md"
              onDeleted={() => void navigate({ to: "/spaces" })}
            />
          ) : (
            <ReminderButton space={space} size="md" />
          )}
          {isModerator(role) && (
            <Button
              variant="secondary"
              icon={<UserPlus className="size-4" />}
              onClick={() => setInviting(true)}
            >
              {t("menu.invite")}
            </Button>
          )}
        </div>
        {space.viewer?.invited && (
          <p className="rounded-2xl bg-brand-soft px-4 py-3 text-sm font-bold">
            💌 {t("scheduled.invited")}
          </p>
        )}
      </Card>
      <InvitesDialog
        space={space}
        moderator={isModerator(role)}
        open={inviting}
        onOpenChange={setInviting}
      />
    </Page>
  );
}

/** A room that is over (or was called off): what it was, and who was there to report or look up. */
export function EndedRoom({ space }: { space: spaces.Space }) {
  const { t } = useTranslation("spaces");
  const navigate = useNavigate();
  const myId = useMe()?.user?.id ?? "";
  const cancelled = space.state === spaces.SpaceState.CANCELLED;
  const host = space.host?.id === myId;
  // Who was there is only told to people who were inside; anyone else gets nothing to list.
  const past = useParticipants(space.id, spaces.ParticipantFilter.PAST, !cancelled);
  const minutes = minutesOf(space);
  const reason =
    space.endReason === spaces.SpaceEndReason.MODERATION
      ? "moderation"
      : space.endReason === spaces.SpaceEndReason.HOST_ENDED
        ? "hostEnded"
        : space.endReason === spaces.SpaceEndReason.NEVER_STARTED
          ? "neverStarted"
          : space.endReason === spaces.SpaceEndReason.EMPTY
            ? "empty"
            : "hostLeft";
  return (
    <Page
      title={t(cancelled ? "ended.cancelledTitle" : "ended.title")}
      back="/spaces"
      actions={
        <SubjectMoreButton
          subject={subjectOf(space)}
          label={space.title}
          share={false}
          report={!host}
        />
      }
    >
      <Card className="flex flex-col gap-4" data-testid="ended-room">
        <div className="text-5xl">{cancelled ? "🫥" : "👋"}</div>
        <div>
          <h2 className="text-2xl leading-snug font-black break-words">
            <EmojiText text={space.title} />
          </h2>
          <p className="mt-1 text-sm font-bold text-ink-soft">{t(`ended.reason.${reason}`)}</p>
          <TagList tags={space.tags} className="mt-2" />
        </div>
        <div className="flex items-center gap-3">
          <Avatar user={space.host} outlined />
          <p className="min-w-0 truncate font-bold">
            <PersonName user={space.host} />
          </p>
        </div>
        {!cancelled && (
          <dl className="grid grid-cols-3 gap-2 text-center">
            {[
              [t("ended.minutes"), t("ended.minutesValue", { count: minutes })],
              [t("ended.peak"), space.peakListeners.toLocaleString()],
              [t("ended.total"), space.totalListeners.toLocaleString()],
            ].map(([label, value]) => (
              <div key={label} className="rounded-2xl bg-surface-2 px-2 py-3">
                <dd className="text-xl font-black">{value}</dd>
                <dt className="text-xs font-bold text-ink-faint">{label}</dt>
              </div>
            ))}
          </dl>
        )}
        {host && (
          <div className="flex flex-wrap gap-2">
            <MySpaceActions
              space={space}
              size="md"
              onDeleted={() => void navigate({ to: "/spaces" })}
            />
          </div>
        )}
      </Card>

      {past.data && past.data.length > 0 && (
        <section className="mt-6">
          <h2 className="text-lg font-black">{t("ended.people")}</h2>
          <p className="mt-1 text-sm text-ink-soft">{t("ended.peopleBody")}</p>
          <ul className="mt-2 flex flex-col">
            {past.data.map((entry) => (
              <PersonRow
                key={entry.user?.id}
                user={entry.user}
                trailing={
                  entry.user &&
                  entry.user.id !== myId && (
                    <ReportButton
                      subject={{ type: common.SubjectType.USER, id: entry.user.id }}
                      label={`@${entry.user.handle}`}
                    />
                  )
                }
              />
            ))}
          </ul>
          <LoadMore query={past.paging} />
        </section>
      )}
    </Page>
  );
}
