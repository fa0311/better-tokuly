import {
  AtSign,
  BookHeart,
  EyeOff,
  HardDriveDownload,
  Link2,
  Palette,
  ShieldCheck,
  UserRound,
  Users,
} from "lucide-react";
import i18n from "#/lib/i18n";
import type { SettingsSection } from "#/seams/settings";
import { useMe } from "#/stores/session";

const INK = "text-[#17122b]";
const strings = (id: string) => ({
  title: () => i18n.t(`settings:sections.${id}.title`),
  hint: () => i18n.t(`settings:sections.${id}.hint`),
});

/**
 * The sections the settings feature owns. Pages of other features (notifications,
 * comments, "use as an app") add their own row from their `settings-section.ts`.
 */
export default [
  {
    id: "profile",
    order: 10,
    to: "/settings/profile",
    icon: UserRound,
    tint: `bg-sky ${INK}`,
    ...strings("profile"),
  },
  {
    id: "profile-book",
    order: 15,
    to: "/settings/profile-book",
    icon: BookHeart,
    tint: `bg-pink ${INK}`,
    ...strings("profileBook"),
  },
  {
    id: "accounts",
    order: 25,
    to: "/settings/accounts",
    icon: Users,
    tint: `bg-sky ${INK}`,
    ...strings("accounts"),
  },
  {
    id: "account",
    order: 20,
    to: "/settings/account",
    icon: AtSign,
    tint: `bg-sun ${INK}`,
    ...strings("account"),
    useAttention: () => {
      const me = useMe();
      return me?.email && !me.emailVerified
        ? i18n.t("settings:home.attention.emailUnverified")
        : undefined;
    },
  },
  {
    id: "security",
    order: 30,
    to: "/settings/security",
    icon: ShieldCheck,
    tint: `bg-mint ${INK}`,
    ...strings("security"),
  },
  {
    id: "privacy",
    order: 40,
    to: "/settings/privacy",
    icon: EyeOff,
    tint: `bg-pink ${INK}`,
    ...strings("privacy"),
  },
  {
    id: "appearance",
    order: 60,
    to: "/settings/appearance",
    icon: Palette,
    tint: `bg-lime ${INK}`,
    ...strings("appearance"),
  },
  {
    id: "connections",
    order: 80,
    to: "/settings/connections",
    icon: Link2,
    tint: `bg-coral ${INK}`,
    ...strings("connections"),
  },
  {
    id: "data",
    order: 90,
    to: "/settings/data",
    icon: HardDriveDownload,
    tint: "bg-brand text-on-brand",
    ...strings("data"),
    useAttention: () =>
      useMe()?.deletionScheduledAt
        ? i18n.t("settings:home.attention.deletionScheduled")
        : undefined,
  },
] satisfies SettingsSection[];
