import { common, identity } from "@tokuly/api";
import { router } from "expo-router";
import { Ban } from "lucide-react-native";
import { useTranslation } from "react-i18next";
import { ChoiceRow, SwitchRow } from "#/components/form-rows";
import { Page, Row, Section, Spinner } from "#/components/page";
import { useMe } from "#/features/session/store";
import { webScreenHref } from "#/features/web/handoff";
import { useTheme } from "#/lib/theme";
import { useChangePrivacy, useSettings } from "./data";

type AudienceName = "EVERYONE" | "FRIENDS_OF_FRIENDS" | "FRIENDS" | "NOBODY";
type AudienceField =
  | "friendRequestsFrom"
  | "discoverableTo"
  | "friendListVisibleTo"
  | "footprintsVisibleTo"
  | "directMessagesFrom"
  | "callsFrom";
type SwitchField =
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
/** Being found has no "friends only": friends have found you already. */
const DISCOVERY_AUDIENCES = REQUEST_AUDIENCES;
/** Under 18, discovery and friend requests reach friends of friends at most. */
const forMinors = (audiences: AudienceName[]) => audiences.filter((name) => name !== "EVERYONE");
/** Who may see the birthday: friends of friends is not offered for it. */
const BIRTHDAY_AUDIENCES: AudienceName[] = ["EVERYONE", "FRIENDS", "NOBODY"];

type VisibilityName = "PUBLIC" | "FOLLOWERS" | "FRIENDS" | "PRIVATE";
const NOTE_VISIBILITIES: VisibilityName[] = ["PUBLIC", "FOLLOWERS", "FRIENDS", "PRIVATE"];

/** How long raw location history is kept, in days; 0 keeps it forever. */
const HISTORY_DAYS = [0, 7, 30, 90, 365];

/**
 * Who may do and see what, as on the web (`settings/privacy`). Every control
 * saves the moment it changes. The blocked list opens on the web.
 */
export function PrivacyScreen() {
  const { t } = useTranslation("settings");
  const { colors } = useTheme();
  const { data } = useSettings();
  const change = useChangePrivacy();
  const minor = useMe()?.minor ?? false;
  const privacy = data?.privacy;
  if (!privacy) return <Spinner />;

  const audience = (field: AudienceField, offered = EVERY_AUDIENCE) => {
    const current = identity.Audience[privacy[field]] as AudienceName;
    // A value set elsewhere that is not offered here still has to be shown.
    const names =
      offered.includes(current) || !EVERY_AUDIENCE.includes(current)
        ? offered
        : [...offered, current];
    return (
      <ChoiceRow<AudienceName>
        testID={`privacy-${field}`}
        label={t(`privacy.${field}.label`)}
        hint={t(`privacy.${field}.hint`)}
        value={names.includes(current) ? current : (names[0] ?? "EVERYONE")}
        onChange={(next) => change({ [field]: identity.Audience[next] })}
        choices={names.map((name) => ({
          value: name,
          label: t(`privacy.audience.${name}`),
          hint: t(`privacy.${field}.options.${name}`),
        }))}
      />
    );
  };
  const toggle = (field: SwitchField) => (
    <SwitchRow
      testID={`privacy-${field}`}
      label={t(`privacy.${field}.label`)}
      hint={t(`privacy.${field}.hint`)}
      value={privacy[field]}
      onChange={(value) => change({ [field]: value })}
    />
  );

  const historyDays = HISTORY_DAYS.includes(privacy.locationHistoryDays)
    ? HISTORY_DAYS
    : [...HISTORY_DAYS, privacy.locationHistoryDays].sort((a, b) => a - b);
  const noteVisibility = common.Visibility[privacy.defaultNoteVisibility] as VisibilityName;
  const storedBirthday = identity.Audience[privacy.birthdayVisibleTo] as AudienceName;
  const birthdayAudience = BIRTHDAY_AUDIENCES.includes(storedBirthday) ? storedBirthday : "NOBODY";

  return (
    <Page testID="privacy-screen">
      <Section
        title={t("privacy.groups.connect.title")}
        lead={minor ? t("privacy.minorLimit") : t("privacy.groups.connect.lead")}
      >
        {audience("friendRequestsFrom", minor ? forMinors(REQUEST_AUDIENCES) : REQUEST_AUDIENCES)}
        {audience("discoverableTo", minor ? forMinors(DISCOVERY_AUDIENCES) : DISCOVERY_AUDIENCES)}
        {toggle("nearbyDiscoverable")}
        {audience("friendListVisibleTo")}
      </Section>

      <Section title={t("privacy.groups.talk.title")}>
        {audience("directMessagesFrom")}
        {audience("callsFrom")}
      </Section>

      <Section title={t("privacy.groups.map.title")} lead={t("privacy.groups.map.lead")}>
        {toggle("shareOnlineStatus")}
        {toggle("shareBattery")}
        {toggle("shareActivity")}
        {toggle("sharePlaceLabels")}
        {toggle("shareNowPlaying")}
      </Section>

      <Section title={t("privacy.groups.history.title")} lead={t("privacy.groups.history.lead")}>
        {toggle("recordHistory")}
        <ChoiceRow<string>
          testID="privacy-locationHistoryDays"
          label={t("privacy.locationHistoryDays.label")}
          hint={t("privacy.locationHistoryDays.hint")}
          value={String(privacy.locationHistoryDays)}
          onChange={(next) => change({ locationHistoryDays: Number(next) })}
          choices={historyDays.map((days) => ({
            value: String(days),
            label:
              days === 0
                ? t("privacy.locationHistoryDays.forever")
                : t("privacy.locationHistoryDays.days", { count: days }),
          }))}
        />
        {audience("footprintsVisibleTo")}
      </Section>

      <Section title={t("privacy.groups.birthday.title")} lead={t("privacy.groups.birthday.lead")}>
        <ChoiceRow<AudienceName>
          testID="privacy-birthdayVisibleTo"
          label={t("privacy.birthdayVisibleTo.label")}
          hint={t("privacy.birthdayVisibleTo.hint")}
          value={birthdayAudience}
          onChange={(next) => change({ birthdayVisibleTo: identity.Audience[next] })}
          choices={BIRTHDAY_AUDIENCES.map((name) => ({
            value: name,
            label: t(`privacy.audience.${name}`),
            hint: t(`privacy.birthdayVisibleTo.options.${name}`),
          }))}
        />
        <SwitchRow
          testID="privacy-birthdayShowYear"
          label={t("privacy.birthdayShowYear.label")}
          hint={t("privacy.birthdayShowYear.hint")}
          value={privacy.birthdayShowYear}
          disabled={birthdayAudience === "NOBODY"}
          onChange={(value) => change({ birthdayShowYear: value })}
        />
      </Section>

      <Section title={t("privacy.groups.other.title")}>
        {toggle("shareViews")}
        <ChoiceRow<VisibilityName>
          testID="privacy-defaultNoteVisibility"
          label={t("privacy.defaultNoteVisibility.label")}
          hint={t("privacy.defaultNoteVisibility.hint")}
          value={
            NOTE_VISIBILITIES.includes(noteVisibility)
              ? noteVisibility
              : // Unset is the server's default (public); an audience this app does not know reads as private.
                privacy.defaultNoteVisibility === common.Visibility.UNSPECIFIED
                ? "PUBLIC"
                : "PRIVATE"
          }
          onChange={(next) => change({ defaultNoteVisibility: common.Visibility[next] })}
          choices={NOTE_VISIBILITIES.map((name) => ({
            value: name,
            label: t(`privacy.visibility.${name}.label`),
            hint: t(`privacy.visibility.${name}.hint`),
          }))}
        />
      </Section>

      <Section>
        <Row
          testID="privacy-blocked"
          icon={<Ban size={20} color={colors.ink} />}
          label={t("privacy.blocked.title")}
          hint={t("privacy.blocked.linkHint")}
          onPress={() => router.push(webScreenHref("/settings/privacy/blocked") as never)}
        />
      </Section>
    </Page>
  );
}
