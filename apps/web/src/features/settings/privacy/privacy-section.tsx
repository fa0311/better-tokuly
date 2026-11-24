import { Link } from "@tanstack/react-router";
import { common, identity } from "@tokuly/api";
import { Ban, ChevronRight } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Skeleton } from "#/components/ui/page";
import { SelectField } from "#/components/ui/select";
import { SwitchRow } from "#/components/ui/switch";
import { useChangeSettings, useSettings } from "../data";
import { Group, SettingsPage } from "../shell/settings-page";

type AudienceName = "EVERYONE" | "FRIENDS_OF_FRIENDS" | "FRIENDS" | "NOBODY";
type AudienceField =
  | "friendRequestsFrom"
  | "friendListVisibleTo"
  | "footprintsVisibleTo"
  | "directMessagesFrom"
  | "callsFrom";
type SwitchField =
  | "discoverable"
  | "nearbyDiscoverable"
  | "shareBattery"
  | "shareActivity"
  | "shareOnlineStatus"
  | "sharePlaceLabels"
  | "shareNowPlaying"
  | "recordHistory"
  | "shareViews";

const EVERY_AUDIENCE: AudienceName[] = ["EVERYONE", "FRIENDS_OF_FRIENDS", "FRIENDS", "NOBODY"];
/** A friend request from someone who already is a friend makes no sense, so "friends" is not offered there. */
const REQUEST_AUDIENCES: AudienceName[] = ["EVERYONE", "FRIENDS_OF_FRIENDS", "NOBODY"];

type VisibilityName = "PUBLIC" | "FRIENDS" | "PRIVATE";
const NOTE_VISIBILITIES: VisibilityName[] = ["PUBLIC", "FRIENDS", "PRIVATE"];

/** How long raw location history is kept, in days; 0 keeps it forever. */
const HISTORY_DAYS = [0, 7, 30, 90, 365];

/**
 * Who may do and see what. Every control saves the moment it changes; the
 * API stores the group as a whole, so each save sends all of it.
 */
export function PrivacySection() {
  const { t } = useTranslation("settings");
  const { data } = useSettings();
  const change = useChangeSettings();
  const privacy = data?.privacy;

  if (!privacy) {
    return (
      <SettingsPage title={t("sections.privacy.title")}>
        {[0, 1, 2].map((i) => (
          <Skeleton key={i} className="mt-7 h-48 first:mt-0" />
        ))}
      </SettingsPage>
    );
  }

  const set = <K extends keyof identity.PrivacySettings>(
    field: K,
    value: identity.PrivacySettings[K],
  ) => change("privacy", { [field]: value } as Partial<identity.PrivacySettings>);

  const audience = (field: AudienceField, offered: AudienceName[] = EVERY_AUDIENCE) => {
    const current = identity.Audience[privacy[field]] as AudienceName;
    // A value set elsewhere that is not offered here still has to be shown.
    const names =
      offered.includes(current) || !EVERY_AUDIENCE.includes(current)
        ? offered
        : [...offered, current];
    return (
      <SelectField<AudienceName>
        className="px-3 py-3"
        label={t(`privacy.${field}.label`)}
        hint={t(`privacy.${field}.hint`)}
        value={names.includes(current) ? current : (names[0] ?? "EVERYONE")}
        onChange={(next) => set(field, identity.Audience[next])}
        options={names.map((name) => ({
          value: name,
          label: t(`privacy.audience.${name}`),
          hint: t(`privacy.${field}.options.${name}`),
        }))}
      />
    );
  };
  const toggle = (field: SwitchField) => (
    <SwitchRow
      label={t(`privacy.${field}.label`)}
      hint={t(`privacy.${field}.hint`)}
      checked={privacy[field]}
      onChange={(checked) => set(field, checked)}
    />
  );

  const historyDays = HISTORY_DAYS.includes(privacy.locationHistoryDays)
    ? HISTORY_DAYS
    : [...HISTORY_DAYS, privacy.locationHistoryDays].sort((a, b) => a - b);
  const noteVisibility = common.Visibility[privacy.defaultNoteVisibility] as VisibilityName;

  return (
    <SettingsPage title={t("sections.privacy.title")}>
      <Group
        title={t("privacy.groups.connect.title")}
        lead={t("privacy.groups.connect.lead")}
        flush
      >
        {audience("friendRequestsFrom", REQUEST_AUDIENCES)}
        {toggle("discoverable")}
        {toggle("nearbyDiscoverable")}
        {audience("friendListVisibleTo")}
      </Group>

      <Group title={t("privacy.groups.talk.title")} flush>
        {audience("directMessagesFrom")}
        {audience("callsFrom")}
      </Group>

      <Group title={t("privacy.groups.map.title")} lead={t("privacy.groups.map.lead")} flush>
        {toggle("shareOnlineStatus")}
        {toggle("shareBattery")}
        {toggle("shareActivity")}
        {toggle("sharePlaceLabels")}
        {toggle("shareNowPlaying")}
      </Group>

      <Group
        title={t("privacy.groups.history.title")}
        lead={t("privacy.groups.history.lead")}
        flush
      >
        {toggle("recordHistory")}
        <SelectField<string>
          className="px-3 py-3"
          label={t("privacy.locationHistoryDays.label")}
          hint={t("privacy.locationHistoryDays.hint")}
          value={String(privacy.locationHistoryDays)}
          onChange={(next) => set("locationHistoryDays", Number(next))}
          options={historyDays.map((days) => ({
            value: String(days),
            label:
              days === 0
                ? t("privacy.locationHistoryDays.forever")
                : t("privacy.locationHistoryDays.days", { count: days }),
          }))}
        />
        {audience("footprintsVisibleTo")}
      </Group>

      <Group title={t("privacy.groups.other.title")} flush>
        {toggle("shareViews")}
        <SelectField<VisibilityName>
          className="px-3 py-3"
          label={t("privacy.defaultNoteVisibility.label")}
          hint={t("privacy.defaultNoteVisibility.hint")}
          value={
            NOTE_VISIBILITIES.includes(noteVisibility)
              ? noteVisibility
              : // Unset is the server's default (public); an audience this client does not know reads as private.
                privacy.defaultNoteVisibility === common.Visibility.UNSPECIFIED
                ? "PUBLIC"
                : "PRIVATE"
          }
          onChange={(next) => set("defaultNoteVisibility", common.Visibility[next])}
          options={NOTE_VISIBILITIES.map((name) => ({
            value: name,
            label: t(`privacy.visibility.${name}.label`),
            hint: t(`privacy.visibility.${name}.hint`),
          }))}
        />
      </Group>

      <Link
        to="/settings/privacy/blocked"
        className="pressable sticker mt-7 flex items-center gap-3 rounded-card bg-surface p-4"
      >
        <span className="grid size-10 shrink-0 place-items-center rounded-xl border-2 border-ink bg-coral text-[#17122b]">
          <Ban className="size-5" />
        </span>
        <span className="min-w-0 flex-1">
          <span className="block font-black">{t("privacy.blocked.title")}</span>
          <span className="block text-sm text-ink-soft">{t("privacy.blocked.linkHint")}</span>
        </span>
        <ChevronRight className="size-5 shrink-0 text-ink-faint" />
      </Link>
    </SettingsPage>
  );
}
