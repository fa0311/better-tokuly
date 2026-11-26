import type { common } from "@tokuly/api";
import { plans } from "@tokuly/api";
import { rsvpKey } from "@tokuly/core/plans";
import { useTranslation } from "react-i18next";
import { Text, View } from "react-native";
import { Avatar } from "#/components/avatar";
import { INK_ON_BRIGHT } from "#/lib/theme";

const RSVP_TONE: Record<number, { box: string; ink?: string }> = {
  [plans.Rsvp.GOING]: { box: "bg-lime", ink: INK_ON_BRIGHT },
  [plans.Rsvp.MAYBE]: { box: "bg-sun", ink: INK_ON_BRIGHT },
  [plans.Rsvp.DECLINED]: { box: "bg-surface-3" },
  [plans.Rsvp.INVITED]: { box: "bg-pink", ink: "#ffffff" },
  [plans.Rsvp.WAITLISTED]: { box: "bg-sky", ink: INK_ON_BRIGHT },
};

/** A small pill: a word on a colour ("中止", "あした"). */
export function Chip({ label, box, ink }: { label: string; box: string; ink?: string }) {
  return (
    <View className={`rounded-full px-2.5 py-0.5 ${box}`}>
      <Text
        numberOfLines={1}
        className={`text-xs font-black ${ink ? "" : "text-ink-soft"}`}
        style={ink ? { color: ink } : undefined}
      >
        {label}
      </Text>
    </View>
  );
}

/** Someone's answer to a plan as a chip ("行く", "未回答"). */
export function RsvpChip({ rsvp }: { rsvp: plans.Rsvp }) {
  const { t } = useTranslation("plans");
  const tone = RSVP_TONE[rsvp] ?? RSVP_TONE[plans.Rsvp.INVITED]!;
  return <Chip label={t(`rsvp.short.${rsvpKey(rsvp)}`)} box={tone.box} ink={tone.ink} />;
}

/** A few faces overlapping, and how many more there are. */
export function AvatarStack({ users, total }: { users: common.UserSummary[]; total: number }) {
  const shown = users.slice(0, 4);
  const more = total - shown.length;
  return (
    <View className="flex-row items-center">
      {shown.map((user, index) => (
        <View key={user.id} style={{ marginLeft: index === 0 ? 0 : -8 }}>
          <Avatar user={user} size="xs" />
        </View>
      ))}
      {more > 0 && <Text className="ml-1.5 text-xs font-black text-ink-soft">+{more}</Text>}
    </View>
  );
}

/** The emoji of a plan on its tile. */
export function PlanEmoji({ emoji, size }: { emoji: string; size: "md" | "lg" }) {
  const box = size === "lg" ? "size-20 rounded-[22px]" : "size-14 rounded-2xl";
  return (
    <View className={`items-center justify-center border-2 border-ink bg-brand-soft ${box}`}>
      <Text className={size === "lg" ? "text-5xl leading-[60px]" : "text-3xl leading-10"}>
        {emoji || "📅"}
      </Text>
    </View>
  );
}
