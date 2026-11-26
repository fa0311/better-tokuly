import { router } from "expo-router";
import { CalendarPlus } from "lucide-react-native";
import { useTranslation } from "react-i18next";
import { Button } from "#/components/button";
import { useTheme } from "#/lib/theme";
import type { ProfileSection } from "#/seams/profile";

function InviteToPlan({ user }: { user: { id: string; handle: string } }) {
  const { t } = useTranslation("mobile");
  const { colors } = useTheme();
  return (
    <Button
      testID="profile-plan-invite"
      variant="secondary"
      icon={<CalendarPlus size={18} color={colors.ink} />}
      onPress={() =>
        router.push(
          `/plans/new?invite=${encodeURIComponent(user.id)}&handle=${encodeURIComponent(user.handle)}`,
        )
      }
    >
      {t("plans.inviteToPlan")}
    </Button>
  );
}

/** On a friend's profile: a new plan with them already invited. */
export default [
  {
    id: "plans",
    slot: "action",
    order: 30,
    Component: ({ user, isFriend, isSelf }) =>
      isFriend && !isSelf ? <InviteToPlan user={user} /> : null,
  },
] satisfies ProfileSection[];
