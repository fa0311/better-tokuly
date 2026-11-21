import type { common } from "@tokuly/api";
import { cn } from "#/lib/cn";

const SIZES = {
  xs: "size-6 rounded-[0.45rem] text-[10px]",
  sm: "size-8 rounded-[0.6rem] text-xs",
  md: "size-11 rounded-[0.85rem] text-sm",
  lg: "size-16 rounded-[1.15rem] text-xl",
  xl: "size-24 rounded-[1.6rem] text-3xl",
} as const;

/** Fallback tile colours, picked from the user ID so a person keeps theirs. */
const TINTS = ["bg-brand text-on-brand", "bg-lime text-[#17122b]", "bg-pink text-white", "bg-sky text-[#17122b]", "bg-sun text-[#17122b]", "bg-coral text-white", "bg-mint text-[#17122b]"];

function tintFor(seed: string): string {
  let hash = 0;
  for (let i = 0; i < seed.length; i++) hash = (hash * 31 + seed.charCodeAt(i)) >>> 0;
  return TINTS[hash % TINTS.length]!;
}

/** Smallest stored variant that still covers `pixels` on a 2x screen. */
function pickUrl(image: common.Image | undefined, pixels: number): string | undefined {
  if (!image) return undefined;
  const wanted = pixels * 2;
  const fitting = image.variants.filter((v) => v.size >= wanted).sort((a, b) => a.size - b.size)[0];
  return fitting?.url ?? image.url ?? undefined;
}

const PIXELS = { xs: 24, sm: 32, md: 44, lg: 64, xl: 96 } as const;

export interface AvatarProps {
  user?: Pick<common.UserSummary, "id" | "displayName" | "handle" | "avatar" | "deleted">;
  size?: keyof typeof SIZES;
  /** Draws the ink outline that makes the avatar a "sticker" (map pins, headers). */
  outlined?: boolean;
  /** Green dot: the person has the app open. */
  online?: boolean;
  className?: string;
}

/**
 * A person's face: a rounded square, like the original Tokuly map marker.
 * Falls back to their initial on a colour derived from their ID.
 */
export function Avatar({ user, size = "md", outlined, online, className }: AvatarProps) {
  const name = user?.displayName || user?.handle || "?";
  const url = user?.deleted ? undefined : pickUrl(user?.avatar, PIXELS[size]);
  return (
    <span className={cn("relative inline-block shrink-0", className)}>
      <span
        className={cn(
          "grid place-items-center overflow-hidden font-black",
          SIZES[size],
          outlined && "sticker-sm",
          !url && (user?.deleted ? "bg-surface-3 text-ink-faint" : tintFor(user?.id ?? name)),
        )}
        style={url && user?.avatar?.dominantColor ? { background: user.avatar.dominantColor } : undefined}
      >
        {url ? (
          <img src={url} alt="" className="size-full object-cover" loading="lazy" decoding="async" draggable={false} />
        ) : (
          <span aria-hidden>{user?.deleted ? "?" : Array.from(name)[0]?.toUpperCase()}</span>
        )}
      </span>
      {online && (
        <span className="absolute -right-0.5 -bottom-0.5 size-3 rounded-full border-2 border-surface bg-success" aria-label="online" />
      )}
    </span>
  );
}
