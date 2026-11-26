import { router } from "expo-router";
import { useTranslation } from "react-i18next";
import { Row, Section } from "#/components/page";
import { ProfileScreen } from "#/features/profile/profile-screen";
import { useMe } from "#/features/session/store";
import { useBadges } from "#/seams/badges";
import { HUB_ENTRIES } from "#/seams/hub";

/** The own profile, with the hub under it: the way into every area that has no tab. */
export default function Me() {
  // The labels are read below in the language of this render.
  useTranslation();
  const me = useMe();
  const badges = useBadges();
  return (
    <ProfileScreen
      handle={me?.user?.handle ?? ""}
      footer={
        <Section>
          {HUB_ENTRIES.map((entry) => (
            <Row
              key={entry.id}
              testID={`me-${entry.id}`}
              label={entry.label()}
              badge={entry.badge ? badges[entry.badge] : undefined}
              onPress={() => router.push(entry.href)}
            />
          ))}
        </Section>
      }
    />
  );
}
