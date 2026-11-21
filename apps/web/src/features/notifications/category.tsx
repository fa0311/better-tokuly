import { Bell, Clapperboard, Coins, Heart, MapPin, MessageCircle, Newspaper, Radio, ShieldCheck, Smile, StickyNote, UserPlus, Zap } from "lucide-react";
import type { ComponentType } from "react";
import { cn } from "#/lib/cn";

interface CategoryLook {
  icon: ComponentType<{ className?: string }>;
  /** Background and text colour of the icon bubble. */
  tint: string;
}

/** How each preference category (`CategoryPreference.category`) looks. */
const LOOKS: Record<string, CategoryLook> = {
  social: { icon: UserPlus, tint: "bg-sky text-[#17122b]" },
  chat: { icon: MessageCircle, tint: "bg-brand text-on-brand" },
  geo: { icon: MapPin, tint: "bg-lime text-[#17122b]" },
  notes: { icon: StickyNote, tint: "bg-sun text-[#17122b]" },
  video: { icon: Clapperboard, tint: "bg-coral text-white" },
  live: { icon: Radio, tint: "bg-live text-white" },
  shorts: { icon: Zap, tint: "bg-pink text-white" },
  engagement: { icon: Heart, tint: "bg-pink text-white" },
  stamp: { icon: Smile, tint: "bg-sun text-[#17122b]" },
  wallet: { icon: Coins, tint: "bg-lime text-[#17122b]" },
  feed: { icon: Newspaper, tint: "bg-mint text-[#17122b]" },
  system: { icon: ShieldCheck, tint: "bg-surface-3 text-ink" },
};

/** Categories this version of the app has no look for still get a bell. */
const FALLBACK: CategoryLook = { icon: Bell, tint: "bg-surface-3 text-ink" };

const SIZES = {
  /** Corner badge on an avatar. */
  xs: { box: "size-5 rounded-full border-2 border-surface", icon: "size-3" },
  sm: { box: "size-8 rounded-[0.6rem]", icon: "size-4" },
  /** Stands in for the avatar when nobody caused the notification. */
  md: { box: "size-11 rounded-[0.85rem]", icon: "size-5" },
} as const;

/** The icon bubble of a notification category. */
export function CategoryIcon({ category, size = "sm", className }: { category: string; size?: keyof typeof SIZES; className?: string }) {
  const look = LOOKS[category] ?? FALLBACK;
  const Icon = look.icon;
  return (
    <span aria-hidden className={cn("grid shrink-0 place-items-center", SIZES[size].box, look.tint, className)}>
      <Icon className={SIZES[size].icon} />
    </span>
  );
}
