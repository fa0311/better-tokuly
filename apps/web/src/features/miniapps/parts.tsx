import type { common } from "@tokuly/api";
import { cn } from "#/lib/cn";

/** A mini app's icon, or its initial on a tile. */
export function AppIcon({
  icon,
  name,
  className,
}: {
  icon?: common.Image;
  name: string;
  className?: string;
}) {
  const url =
    [...(icon?.variants ?? [])].sort((a, b) => a.size - b.size).find((v) => v.size >= 96)?.url ??
    icon?.url;
  return (
    <span
      className={cn(
        "grid size-10 shrink-0 place-items-center overflow-hidden rounded-xl border-2 border-ink bg-sky font-black text-[#17122b]",
        className,
      )}
    >
      {url ? (
        <img src={url} alt="" className="size-full object-cover" draggable={false} />
      ) : (
        <span aria-hidden>{Array.from(name.trim() || "?")[0]?.toUpperCase()}</span>
      )}
    </span>
  );
}
