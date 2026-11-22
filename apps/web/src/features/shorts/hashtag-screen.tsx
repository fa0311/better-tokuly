import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { errorReason, shorts } from "@tokuly/api";
import { Flame, Hash } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { EmptyState, ListRow, Page, Skeleton } from "#/components/ui/page";
import { compactCount } from "#/features/engagement";
import { errorMessage } from "#/lib/errors";
import { discoveryClient, shortKeys } from "./api";
import { ShortList } from "./short-tile";
import { ChoiceChips } from "./sort-chips";

const { HashtagSort } = shorts;

/** `/shorts/tags/<tag>`: the public shorts carrying a hashtag, the best first or the newest. */
export function HashtagScreen({ tag }: { tag: string }) {
  const { t, i18n } = useTranslation("shorts");
  const [sort, setSort] = useState<shorts.HashtagSort>(HashtagSort.TOP);
  const name = tag.replace(/^[#＃]/, "").toLowerCase();
  const hashtag = useQuery({
    queryKey: shortKeys.hashtag(name),
    queryFn: async () => (await discoveryClient().getHashtag({ tag: name })).hashtag!,
    retry: false,
  });
  const info = hashtag.data;
  // A tag nothing public carries is simply an empty page; anything else is an error to show.
  const missing = hashtag.isError && errorReason(hashtag.error) === "HASHTAG_NOT_FOUND";

  return (
    <Page title={`#${name}`} back="/shorts/tags" wide>
      {hashtag.isError && !missing ? (
        <EmptyState emoji="😵‍💫" title={errorMessage(hashtag.error, { namespace: "shorts" })} />
      ) : (
        <>
          <div className="mb-4 flex flex-wrap items-center gap-x-4 gap-y-2">
            <span className="sticker grid size-14 shrink-0 place-items-center rounded-3xl bg-sky text-[#17122b]">
              <Hash className="size-7" />
            </span>
            <div className="min-w-0 flex-1">
              <p className="text-sm font-bold text-ink-soft" data-testid="hashtag-counts">
                {info
                  ? t("hashtag.counts", {
                      shorts: compactCount(Number(info.shortCount), i18n.language),
                      views: compactCount(Number(info.viewCount), i18n.language),
                    })
                  : missing
                    ? t("hashtag.counts", { shorts: 0, views: 0 })
                    : "…"}
              </p>
              {info && info.trendingRank > 0 && (
                <p className="mt-1 inline-flex items-center gap-1 rounded-full bg-sun px-2.5 py-0.5 text-xs font-black text-[#17122b]">
                  <Flame className="size-3.5" aria-hidden />
                  {t("hashtag.trending", { rank: info.trendingRank })}
                </p>
              )}
            </div>
            <ChoiceChips
              label={t("hashtag.sortLabel")}
              value={sort}
              onChange={(next) => setSort(next)}
              options={[
                { value: HashtagSort.TOP, label: t("hashtag.sort.top") },
                { value: HashtagSort.RECENT, label: t("hashtag.sort.recent") },
              ]}
            />
          </div>
          <ShortList
            context={{ kind: "hashtag", key: name }}
            options={{ sort }}
            empty={
              <EmptyState
                emoji="🏷️"
                title={t("hashtag.empty", { tag: name })}
                body={t("hashtag.emptyBody")}
                action={
                  <Link
                    to="/shorts/new"
                    search={{ tag: name }}
                    className="pressable sticker inline-flex h-11 items-center rounded-full bg-brand px-5 font-bold text-on-brand"
                  >
                    {t("hashtag.post")}
                  </Link>
                }
              />
            }
          />
        </>
      )}
    </Page>
  );
}

/** `/shorts/tags`: the hashtags that are being used and watched most right now. */
export function TrendingTagsScreen() {
  const { t, i18n } = useTranslation("shorts");
  const query = useQuery({
    queryKey: shortKeys.trendingHashtags,
    queryFn: async () => (await discoveryClient().listTrendingHashtags({ limit: 50 })).hashtags,
  });
  return (
    <Page title={t("tags.title")} back="/shorts">
      {query.isPending ? (
        <div className="flex flex-col gap-2">
          {Array.from({ length: 6 }, (_, index) => (
            <Skeleton key={index} className="h-16" />
          ))}
        </div>
      ) : query.isError ? (
        <EmptyState emoji="😵‍💫" title={errorMessage(query.error, { namespace: "shorts" })} />
      ) : query.data.length === 0 ? (
        <EmptyState emoji="🌱" title={t("tags.empty")} body={t("tags.emptyBody")} />
      ) : (
        <ol className="flex flex-col gap-1">
          {query.data.map((hashtag, index) => (
            <li key={hashtag.tag}>
              <Link to="/shorts/tags/$tag" params={{ tag: hashtag.tag }} className="block">
                <ListRow
                  className="hover:bg-surface-2"
                  leading={
                    <span className="grid size-11 shrink-0 place-items-center rounded-2xl border-2 border-ink bg-sun font-black text-[#17122b]">
                      {index + 1}
                    </span>
                  }
                  title={`#${hashtag.tag}`}
                  subtitle={t("hashtag.counts", {
                    shorts: compactCount(Number(hashtag.shortCount), i18n.language),
                    views: compactCount(Number(hashtag.viewCount), i18n.language),
                  })}
                />
              </Link>
            </li>
          ))}
        </ol>
      )}
    </Page>
  );
}
