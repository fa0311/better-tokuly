import { Link } from "@tanstack/react-router";
import type { common, identity } from "@tokuly/api";
import { cn } from "#/lib/cn";
import { useTranslation } from "#/lib/use-translation";
import { CheerButton } from "./study-social";

const STYLE: Record<string, { emoji: string; className: string }> = {
  "school.studying": { emoji: "📖", className: "bg-brand-soft" },
  "school.now": { emoji: "🏫", className: "bg-surface" },
  "school.exam": { emoji: "📝", className: "bg-sun text-[#17122b]" },
};

/**
 * The school lines of a profile: "勉強中" (with a cheer button for friends),
 * the class the person is in or has next, and "テスト期間" with its
 * countdown. The server decides which of them the viewer may see; nothing is
 * drawn when it sent none.
 */
export function SchoolProfileCards({
  cards,
  user,
  isSelf,
}: {
  cards: identity.ProfileCard[];
  user: Pick<common.UserSummary, "id" | "displayName">;
  isSelf: boolean;
}) {
  const { i18n } = useTranslation("school");
  const school = cards.filter((card) => card.kind in STYLE).sort((a, b) => a.order - b.order);
  if (school.length === 0) return null;
  const pick = (text: common.LocalizedText | undefined) =>
    (i18n.language.startsWith("ja") ? text?.ja : text?.en) || text?.ja || "";
  return (
    <ul className="mt-5 flex flex-col gap-2">
      {school.map((card) => {
        const style = STYLE[card.kind];
        const resting = card.data.resting === "true";
        const body = (
          <>
            <span className="text-2xl" aria-hidden>
              {card.kind === "school.studying" && resting ? "☕" : style?.emoji}
            </span>
            <span className="min-w-0 flex-1">
              <span className="block text-xs font-black opacity-70">{pick(card.title)}</span>
              <span className="block truncate font-black">{pick(card.text)}</span>
            </span>
          </>
        );
        return (
          <li
            key={card.kind}
            data-kind={card.kind}
            className={cn(
              "sticker-sm flex items-center gap-3 rounded-2xl px-4 py-2.5",
              style?.className,
            )}
          >
            {card.path && (isSelf || card.kind !== "school.studying" || card.data.room_id) ? (
              <Link to={card.path} className="flex min-w-0 flex-1 items-center gap-3">
                {body}
              </Link>
            ) : (
              <span className="flex min-w-0 flex-1 items-center gap-3">{body}</span>
            )}
            {card.kind === "school.studying" && !isSelf && (
              <CheerButton userId={user.id} name={user.displayName} />
            )}
          </li>
        );
      })}
    </ul>
  );
}
