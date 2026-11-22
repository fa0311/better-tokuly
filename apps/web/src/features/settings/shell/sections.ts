import { AtSign, Bell, EyeOff, HardDriveDownload, Link2, Palette, ShieldCheck, Smartphone, UserRound } from "lucide-react";
import type { ComponentType } from "react";

export type SectionId = "profile" | "account" | "security" | "privacy" | "notifications" | "appearance" | "app" | "connections" | "data";

export interface Section {
  id: SectionId;
  /** Notification settings and "use as an app" are pages of their own features; the rest live under /settings. */
  to: `/settings/${Exclude<SectionId, "notifications" | "app">}` | "/notifications/settings" | "/pwa";
  icon: ComponentType<{ className?: string }>;
  /** Colours of the icon bubble. */
  tint: string;
  /** Where the row's `title` and `hint` are when another feature's strings hold them; `sections.<id>` otherwise. */
  strings?: string;
}

const INK = "text-[#17122b]";

/** The settings sections, in the order they are listed. Labels are `sections.<id>` in the strings. */
export const SECTIONS: Section[] = [
  { id: "profile", to: "/settings/profile", icon: UserRound, tint: `bg-sky ${INK}` },
  { id: "account", to: "/settings/account", icon: AtSign, tint: `bg-sun ${INK}` },
  { id: "security", to: "/settings/security", icon: ShieldCheck, tint: `bg-mint ${INK}` },
  { id: "privacy", to: "/settings/privacy", icon: EyeOff, tint: `bg-pink ${INK}` },
  { id: "notifications", to: "/notifications/settings", icon: Bell, tint: "bg-surface text-ink" },
  { id: "appearance", to: "/settings/appearance", icon: Palette, tint: `bg-lime ${INK}` },
  { id: "app", to: "/pwa", icon: Smartphone, tint: "bg-ink text-surface", strings: "pwa:settingsRow" },
  { id: "connections", to: "/settings/connections", icon: Link2, tint: `bg-coral ${INK}` },
  { id: "data", to: "/settings/data", icon: HardDriveDownload, tint: "bg-brand text-on-brand" },
];
