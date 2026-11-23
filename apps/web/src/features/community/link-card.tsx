import { ChevronRight } from "lucide-react";
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { cn } from "#/lib/cn";
import { NS, useCommunity } from "./api";
import { communityHref } from "./model";
import { CommunityIcon } from "./parts";

/**
 * A community a chat message links to (`/g/<handle>`): its picture, name and
 * size, read with the viewer's own permissions. While it loads, and when
 * the viewer may not see it (a hidden community they are not in), it shows
 * `fallback`: the plain link, which names nothing.
 *
 *   <CommunityLinkCard handle="neko" onOpen={(path) => open(path)} fallback={<PlainLink />} />
 */
export function CommunityLinkCard({
  handle,
  onOpen,
  fallback,
  className,
}: {
  handle: string;
  onOpen(path: string): void;
  fallback: ReactNode;
  className?: string;
}) {
  const { t } = useTranslation(NS);
  const query = useCommunity(handle);
  const found = query.data;
  if (!found) return fallback;
  return (
    <button
      type="button"
      onClick={() => onOpen(communityHref(found.handle))}
      className={cn(
        "flex w-72 max-w-full items-center gap-3 rounded-2xl border-2 border-line bg-surface p-3 text-left text-ink",
        className,
      )}
      data-testid="community-link-card"
    >
      <CommunityIcon community={found} size="md" />
      <span className="min-w-0 flex-1">
        <span className="block text-[11px] font-bold text-ink-faint">{t("title")}</span>
        <span className="block truncate font-black">{found.name}</span>
        <span className="block truncate text-xs text-ink-soft">
          {t("memberCount", {
            count: found.memberCount,
            formatted: found.memberCount.toLocaleString(),
          })}
        </span>
      </span>
      <ChevronRight className="size-5 shrink-0 text-ink-faint" />
    </button>
  );
}
