import {
  CalendarDays,
  Code,
  GraduationCap,
  type LucideIcon,
  MessageCircleQuestion,
  Mic,
  NotebookPen,
  Radio,
  Settings,
  ShieldCheck,
  SmilePlus,
  Sticker,
  Trophy,
  Tv,
  UsersRound,
  Wallet,
} from "lucide-react-native";

/** A part of Tokuly the app shows through the web for now, and where it starts on the web. */
export interface WebFeature {
  /** Its name under `mobile:more.features`. */
  key: string;
  path: string;
  icon: LucideIcon;
  /** Only for accounts holding a staff role. */
  staff?: boolean;
}

/**
 * Everything the web has and the app does not yet, in the order of the hub.
 * A feature that becomes native leaves this list (and gets its screen in
 * `features/notifications/target.ts`).
 */
export const WEB_FEATURES: readonly WebFeature[] = [
  { key: "notes", path: "/notes", icon: NotebookPen },
  { key: "video", path: "/watch", icon: Tv },
  { key: "live", path: "/live", icon: Radio },
  { key: "spaces", path: "/spaces", icon: Mic },
  { key: "community", path: "/communities", icon: UsersRound },
  { key: "school", path: "/school", icon: GraduationCap },
  { key: "plans", path: "/plans", icon: CalendarDays },
  { key: "ask", path: "/ask", icon: MessageCircleQuestion },
  { key: "quest", path: "/quest", icon: Trophy },
  { key: "emoji", path: "/emoji", icon: SmilePlus },
  { key: "stamps", path: "/stamps", icon: Sticker },
  { key: "wallet", path: "/wallet", icon: Wallet },
  { key: "settings", path: "/settings", icon: Settings },
  { key: "dev", path: "/developers", icon: Code },
  { key: "admin", path: "/admin", icon: ShieldCheck, staff: true },
];

/** The features `roles` may see (an account's roles are its staff roles). */
export function featuresFor(roles: readonly string[]): readonly WebFeature[] {
  const staff = roles.length > 0;
  return WEB_FEATURES.filter((feature) => staff || !feature.staff);
}
