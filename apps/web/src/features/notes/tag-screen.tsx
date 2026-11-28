import { normalizeHashtag } from "@tokuly/core/notes";
import { Hash } from "lucide-react";
import { useRef } from "react";
import { Button } from "#/components/ui/button";
import { EmptyState } from "#/components/ui/page";
import { useTranslation } from "#/lib/use-translation";
import { useTimeline } from "./api";
import { openCompose } from "./compose-store";
import { ComposeButton, NotesPage } from "./shell";
import { Timeline, type TimelineHandle } from "./timeline";
import { TrendingHashtags } from "./trending";

/** Everything posted with one hashtag, newest first, and a shortcut to join in. */
export function TagScreen({ tag: raw }: { tag: string }) {
  const { t } = useTranslation("notes");
  const timeline = useRef<TimelineHandle>(null);
  const tag = normalizeHashtag(raw);
  const query = useTimeline({ kind: "hashtag", tag: tag ?? "" }, Boolean(tag));

  if (!tag) {
    return (
      <NotesPage title={t("tag.title")} back="/notes" scrollId="tag-invalid">
        <EmptyState emoji="🤔" title={t("tag.invalid")} body={t("tag.invalidBody")} />
      </NotesPage>
    );
  }

  const count = Number(query.data?.pages[0]?.hashtag?.noteCount ?? 0n);
  const context = { text: `#${tag} ` };
  return (
    <NotesPage
      title={`#${tag}`}
      back="/notes"
      scrollId={`tag-${tag}`}
      onRefresh={() => timeline.current?.refresh() ?? Promise.resolve()}
      aside={
        <div className="sticky top-20">
          <TrendingHashtags layout="panel" />
        </div>
      }
    >
      <div className="sticker mb-3 flex items-center gap-3 rounded-card bg-surface p-4">
        <span className="grid size-12 shrink-0 place-items-center rounded-2xl border-2 border-ink bg-sun text-[#17122b]">
          <Hash className="size-6" strokeWidth={3} />
        </span>
        <div className="min-w-0 flex-1">
          <p className="truncate text-lg font-black">#{tag}</p>
          <p className="text-sm font-bold text-ink-soft">{t("tag.count", { count })}</p>
        </div>
        <Button variant="lime" size="sm" onClick={() => openCompose(context)}>
          {t("tag.post")}
        </Button>
      </div>
      <Timeline
        ref={timeline}
        source={{ kind: "hashtag", tag }}
        empty={
          <EmptyState
            emoji="🏷️"
            title={t("tag.empty", { tag })}
            body={t("tag.emptyBody")}
            action={<Button onClick={() => openCompose(context)}>{t("tag.first")}</Button>}
          />
        }
      />
      <ComposeButton context={context} />
    </NotesPage>
  );
}
