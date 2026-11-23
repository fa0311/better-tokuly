import { type common, spaces } from "@tokuly/api";
import { Globe, Lock, Users } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Avatar } from "#/components/ui/avatar";
import { EmojiText } from "#/features/emoji";
import { cn } from "#/lib/cn";
import { nameOf } from "./model";

/** The red "LIVE" pill of a room that is on right now. */
export function LiveBadge({ className }: { className?: string }) {
  const { t } = useTranslation("spaces");
  return (
    <span
      className={cn(
        "inline-flex h-6 shrink-0 items-center gap-1.5 rounded-full bg-live px-2.5 text-[11px] font-black tracking-wide text-white",
        className,
      )}
    >
      <span className="size-1.5 animate-pulse rounded-full bg-white" />
      {t("live")}
    </span>
  );
}

const AUDIENCE_ICON = {
  [spaces.SpaceAudience.PUBLIC]: Globe,
  [spaces.SpaceAudience.FRIENDS]: Users,
  [spaces.SpaceAudience.INVITED]: Lock,
} as const;

export function audienceKey(audience: spaces.SpaceAudience): "public" | "friends" | "invited" {
  if (audience === spaces.SpaceAudience.PUBLIC) return "public";
  if (audience === spaces.SpaceAudience.INVITED) return "invited";
  return "friends";
}

/** Who may enter, as a small label. */
export function AudienceLabel({
  audience,
  className,
}: {
  audience: spaces.SpaceAudience;
  className?: string;
}) {
  const { t } = useTranslation("spaces");
  const key = audienceKey(audience);
  const Icon =
    AUDIENCE_ICON[audience as keyof typeof AUDIENCE_ICON] ??
    AUDIENCE_ICON[spaces.SpaceAudience.FRIENDS];
  return (
    <span className={cn("inline-flex items-center gap-1 text-xs font-bold", className)}>
      <Icon className="size-3.5" />
      {t(`audience.${key}`)}
    </span>
  );
}

/** A room's tags; with `onPick` each one is a button (a filter). */
export function TagList({
  tags,
  onPick,
  className,
}: {
  tags: string[];
  onPick?(tag: string): void;
  className?: string;
}) {
  if (tags.length === 0) return null;
  const pill = "rounded-full bg-surface-2 px-2.5 py-0.5 text-xs font-bold text-ink-soft";
  return (
    <ul className={cn("flex flex-wrap gap-1.5", className)}>
      {tags.map((tag) => (
        <li key={tag}>
          {onPick ? (
            <button
              type="button"
              onClick={() => onPick(tag)}
              className={cn(pill, "hover:text-ink")}
            >
              #{tag}
            </button>
          ) : (
            <span className={pill}>#{tag}</span>
          )}
        </li>
      ))}
    </ul>
  );
}

/** A few faces, overlapping, with "+n" for the rest. */
export function Faces({
  users,
  more = 0,
  size = "sm",
  className,
}: {
  users: common.UserSummary[];
  /** People beyond the faces shown. */
  more?: number;
  size?: "xs" | "sm";
  className?: string;
}) {
  if (users.length === 0 && more <= 0) return null;
  return (
    <span className={cn("flex items-center", className)}>
      {users.map((user, index) => (
        <Avatar
          key={user.id}
          user={user}
          size={size}
          outlined
          className={cn(index > 0 && (size === "xs" ? "-ml-1.5" : "-ml-2"))}
        />
      ))}
      {more > 0 && <span className="ml-1.5 text-xs font-black text-ink-soft">+{more}</span>}
    </span>
  );
}

/** A person's display name, with their custom emoji drawn. */
export function PersonName({ user }: { user: common.UserSummary | undefined }) {
  return <EmojiText text={nameOf(user)} />;
}
