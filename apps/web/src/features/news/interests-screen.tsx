import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { Flame, Heart, RotateCcw, ThumbsDown } from "lucide-react";
import { type ReactNode, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { SwitchRow } from "#/components/ui/switch";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { newsClient, newsKeys, useNewsTags } from "./api";

/** The strongest interest the API keeps (scores stay within ±20). */
const MAX_SCORE = 20;

/**
 * `/news/interests`: what the recommendations are made of — the tags the
 * reader leans towards and away from, what is common right now — and the
 * button that starts them over.
 */
export function InterestsScreen() {
  const { t } = useTranslation("news");
  const queryClient = useQueryClient();
  const tags = useNewsTags(30);
  const [confirming, setConfirming] = useState(false);
  const [clearHistory, setClearHistory] = useState(false);
  const reset = useMutation({
    mutationFn: () => newsClient().resetNewsInterests({ clearHistory }),
    onSuccess: () => {
      toast.success(t("interests.resetDone"));
      return queryClient.invalidateQueries({ queryKey: newsKeys.all });
    },
    onError: (error) => toast.error(errorMessage(error)),
    onSettled: () => setConfirming(false),
  });
  const interests = tags.data?.interests ?? [];
  const avoided = tags.data?.avoided ?? [];
  const popular = tags.data?.popular ?? [];

  return (
    <Page title={t("interests.title")} back="/news">
      <p className="mb-5 px-1 text-sm leading-relaxed text-ink-soft">{t("interests.lead")}</p>
      {tags.isPending ? (
        <div className="flex flex-col gap-3" aria-busy>
          <Skeleton className="h-28" />
          <Skeleton className="h-28" />
        </div>
      ) : tags.isError ? (
        <EmptyState emoji="🌀" title={errorMessage(tags.error)} />
      ) : (
        <div className="flex flex-col gap-6">
          <Group
            title={t("interests.liked")}
            icon={<Heart className="size-5 fill-pink text-pink" />}
            empty={t("interests.likedEmpty")}
          >
            {interests.map((interest) => (
              <ScoreRow
                key={interest.tag}
                tag={interest.tag}
                score={interest.score}
                tone="bg-lime"
              />
            ))}
          </Group>
          <Group
            title={t("interests.avoided")}
            icon={<ThumbsDown className="size-5 text-coral" />}
            empty={t("interests.avoidedEmpty")}
          >
            {avoided.map((interest) => (
              <ScoreRow
                key={interest.tag}
                tag={interest.tag}
                score={interest.score}
                tone="bg-coral"
              />
            ))}
          </Group>
          {popular.length > 0 && (
            <section aria-label={t("interests.popular")}>
              <h2 className="mb-2 flex items-center gap-2 px-1 text-lg font-black">
                <Flame className="size-5 text-coral" />
                {t("interests.popular")}
              </h2>
              <ul className="flex flex-wrap gap-2">
                {popular.map((entry) => (
                  <li key={entry.tag} className="max-w-full">
                    <Link
                      to="/news"
                      search={{ tag: entry.tag }}
                      className="pressable sticker-sm inline-flex h-9 max-w-full items-center gap-1.5 rounded-full bg-surface px-3.5 text-sm font-bold"
                    >
                      <span className="truncate">#{entry.tag}</span>
                      <span className="shrink-0 text-xs text-ink-faint tabular-nums">
                        {entry.articleCount}
                      </span>
                    </Link>
                  </li>
                ))}
              </ul>
            </section>
          )}
          <section className="rounded-card bg-surface p-4">
            <h2 className="font-black">{t("interests.resetTitle")}</h2>
            <p className="mt-0.5 mb-3 text-sm leading-relaxed text-ink-soft">
              {t("interests.resetLead")}
            </p>
            <Button
              variant="secondary"
              icon={<RotateCcw className="size-4" />}
              onClick={() => setConfirming(true)}
            >
              {t("interests.reset")}
            </Button>
          </section>
        </div>
      )}
      <ConfirmDialog
        open={confirming}
        onOpenChange={setConfirming}
        title={t("interests.resetConfirmTitle")}
        description={t("interests.resetConfirmBody")}
        confirmLabel={t("interests.reset")}
        danger
        loading={reset.isPending}
        onConfirm={() => reset.mutate()}
      >
        <SwitchRow
          label={t("interests.clearHistory")}
          hint={t("interests.clearHistoryHint")}
          checked={clearHistory}
          onChange={setClearHistory}
        />
      </ConfirmDialog>
    </Page>
  );
}

function Group({
  title,
  icon,
  empty,
  children,
}: {
  title: string;
  icon: ReactNode;
  empty: string;
  children: ReactNode[];
}) {
  return (
    <section aria-label={title}>
      <h2 className="mb-2 flex items-center gap-2 px-1 text-lg font-black">
        {icon}
        {title}
      </h2>
      {children.length === 0 ? (
        <p className="px-1 text-sm leading-relaxed text-ink-soft">{empty}</p>
      ) : (
        <ul className="flex flex-col gap-1.5">{children}</ul>
      )}
    </section>
  );
}

/** A tag with a bar for how strongly the reader leans towards (or away from) it. */
function ScoreRow({ tag, score, tone }: { tag: string; score: number; tone: string }) {
  const { t } = useTranslation("news");
  const share = Math.min(1, Math.abs(score) / MAX_SCORE);
  return (
    <li>
      <Link
        to="/news"
        search={{ tag }}
        className="flex items-center gap-3 rounded-2xl border-2 border-line bg-surface px-3 py-2.5 transition-colors hover:border-ink-faint"
      >
        <span className="w-2/5 min-w-0 shrink-0 truncate font-bold">#{tag}</span>
        <span
          className="h-3 min-w-0 flex-1 overflow-hidden rounded-full border-2 border-ink bg-surface-3"
          aria-hidden
        >
          <span
            className={cn("block h-full rounded-full", tone)}
            style={{ width: `${Math.max(6, share * 100)}%` }}
          />
        </span>
        <span className="sr-only">
          {t("interests.strength", { percent: Math.round(share * 100) })}
        </span>
      </Link>
    </li>
  );
}
