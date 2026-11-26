import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { notesKeys } from "@tokuly/core/notes";
import { TrendingUp } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Skeleton } from "#/components/ui/page";
import { cn } from "#/lib/cn";
import { noteClient, useTrendingHashtags } from "./api";
import { NoteCard } from "./note-card";

const RANK_TINT = ["bg-sun", "bg-lime", "bg-sky"];

/**
 * Hashtags many different people are using right now. `panel` is the card
 * in the side column of wide screens; `strip` is the row of chips phones
 * scroll sideways. Renders nothing when nothing is trending.
 */
export function TrendingHashtags({ layout }: { layout: "panel" | "strip" }) {
  const { t } = useTranslation("notes");
  const trending = useTrendingHashtags();
  const hashtags =
    trending.data?.hashtags.flatMap((entry) =>
      entry.hashtag ? [{ ...entry, hashtag: entry.hashtag }] : [],
    ) ?? [];

  if (layout === "strip") {
    if (hashtags.length === 0) return null;
    return (
      <nav
        aria-label={t("trending.title")}
        className="scrollbar-none -mx-3 mb-2.5 flex items-center gap-1.5 overflow-x-auto px-3 sm:-mx-4 sm:px-4"
      >
        <span className="flex shrink-0 items-center gap-1 text-xs font-black text-coral">
          <TrendingUp className="size-4" />
          {t("trending.short")}
        </span>
        {hashtags.map((entry) => (
          <Link
            key={entry.hashtag.tag}
            to="/tag/$tag"
            params={{ tag: entry.hashtag.tag }}
            className="shrink-0 rounded-full border-2 border-line bg-surface px-3 py-1 text-sm font-black hover:border-ink-faint"
          >
            #{entry.hashtag.tag}
          </Link>
        ))}
      </nav>
    );
  }

  return (
    <section
      className="sticker rounded-card bg-surface p-4"
      aria-label={t("trending.title")}
      data-testid="trending"
    >
      <h2 className="flex items-center gap-2 text-lg font-black">
        <TrendingUp className="size-5 text-coral" />
        {t("trending.title")}
      </h2>
      {trending.isPending ? (
        <div className="mt-3 flex flex-col gap-2">
          <Skeleton className="h-10" />
          <Skeleton className="h-10" />
          <Skeleton className="h-10" />
        </div>
      ) : hashtags.length === 0 ? (
        <p className="mt-3 text-sm leading-relaxed text-ink-soft">{t("trending.empty")}</p>
      ) : (
        <ol className="mt-2 flex flex-col">
          {hashtags.map((entry, index) => (
            <li key={entry.hashtag.tag}>
              <Link
                to="/tag/$tag"
                params={{ tag: entry.hashtag.tag }}
                className="-mx-2 flex items-center gap-3 rounded-2xl px-2 py-2 hover:bg-surface-2"
              >
                <span
                  className={cn(
                    "grid size-7 shrink-0 place-items-center rounded-lg border-2 border-ink text-xs font-black text-[#17122b]",
                    RANK_TINT[index] ?? "bg-surface-2 text-ink",
                  )}
                >
                  {entry.rank || index + 1}
                </span>
                <span className="min-w-0 flex-1">
                  <span className="block truncate font-black">#{entry.hashtag.tag}</span>
                  <span className="block text-xs font-bold text-ink-faint">
                    {t("trending.count", { count: Number(entry.recentNoteCount) })}
                  </span>
                </span>
              </Link>
            </li>
          ))}
        </ol>
      )}
    </section>
  );
}

/** The language of the trends this reader sees: their app language when it has its own ranking. */
function trendLanguage(language: string): string {
  return language.startsWith("ja") ? "ja" : language.startsWith("en") ? "en" : "";
}

/** Notes getting attention right now, in the reader's language. Renders nothing when there are none. */
export function TrendingNotes() {
  const { t, i18n } = useTranslation("notes");
  const language = trendLanguage(i18n.language);
  const query = useQuery({
    queryKey: notesKeys.trendingNotes(language),
    queryFn: () => noteClient().listTrendingNotes({ language, limit: 5 }),
    staleTime: 60_000,
  });
  const trending = query.data?.notes ?? [];
  if (trending.length === 0) return null;
  return (
    <section aria-label={t("trending.notes")} className="mb-3" data-testid="trending-notes">
      <h2 className="mb-2 flex items-center gap-2 text-sm font-black text-coral">
        <TrendingUp className="size-4" />
        {t("trending.notes")}
      </h2>
      <div className="flex flex-col gap-2.5">
        {trending.map((note) => (
          <NoteCard key={note.id} note={note} />
        ))}
      </div>
    </section>
  );
}
