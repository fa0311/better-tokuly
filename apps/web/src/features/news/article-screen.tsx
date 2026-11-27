import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { Code, errorReason, feed, isApiError } from "@tokuly/api";
import {
  BookOpen,
  Dices,
  ExternalLink,
  Heart,
  MessageCircle,
  MoreHorizontal,
  Share2,
  Trash2,
} from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { Menu } from "#/components/ui/menu";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { useMe } from "#/stores/session";
import { dropArticle, newsAdminClient, newsClient, newsKeys, recordClick } from "./api";
import { ArticlePicture, FeedbackButtons, PublishedAt, SourceIcon, TagLink } from "./parts";

/** The host of an address, for "read on example.com". */
function hostOf(url: string): string {
  try {
    return new URL(url).host.replace(/^www\./, "");
  } catch {
    return "";
  }
}

/** Only http(s) addresses are opened: an article's URL comes from a feed. */
function safeUrl(url: string): string | undefined {
  try {
    const parsed = new URL(url);
    return parsed.protocol === "http:" || parsed.protocol === "https:" ? parsed.href : undefined;
  } catch {
    return undefined;
  }
}

/**
 * `/news/$articleId`: what the app knows about an article — excerpt, source,
 * tags, why it was shown — with the way out to the full text and the reader's
 * feedback. Opening it is what counts as reading: the click is recorded here.
 */
export function ArticleScreen({ articleId }: { articleId: string }) {
  const { t } = useTranslation("news");
  const queryClient = useQueryClient();
  const detail = useQuery({
    queryKey: newsKeys.article(articleId),
    queryFn: () => newsClient().getNewsArticle({ articleId }),
  });
  const article = detail.data?.article;

  // Once per visit of the page, as soon as the article is known to exist.
  const loadedId = article?.id;
  const alreadyClicked = article?.clicked ?? false;
  // `alreadyClicked` is deliberately not a dependency: recording flips it, and must not record again.
  useEffect(() => {
    if (loadedId) recordClick(queryClient, { id: loadedId, clicked: alreadyClicked });
  }, [loadedId, queryClient]);

  if (detail.isPending) {
    return (
      <Page title={t("article.title")} back="/news">
        <div className="flex flex-col gap-4" aria-busy>
          <Skeleton className="aspect-video w-full" />
          <Skeleton className="h-8 w-4/5" />
          <Skeleton className="h-24" />
        </div>
      </Page>
    );
  }
  if (!article) {
    const gone =
      isApiError(detail.error) &&
      (detail.error.code === Code.NotFound || errorReason(detail.error) === "INVALID_ID");
    return (
      <Page title={t("article.title")} back="/news">
        <EmptyState
          emoji={gone ? "🫥" : "🌀"}
          title={gone ? t("article.goneTitle") : errorMessage(detail.error, { namespace: "news" })}
          body={gone ? t("article.goneBody") : undefined}
          action={
            <Link
              to="/news"
              className="pressable sticker inline-flex h-11 items-center rounded-full bg-brand px-5 font-bold text-on-brand"
            >
              {t("article.backToNews")}
            </Link>
          }
        />
      </Page>
    );
  }

  const url = safeUrl(article.url);
  const share = async () => {
    const link = `${window.location.origin}/news/${article.id}`;
    if (navigator.share) {
      await navigator.share({ title: article.title, url: link }).catch(() => {});
      return;
    }
    await navigator.clipboard.writeText(link);
    toast.success(t("article.linkCopied"));
  };

  return (
    <Page title={t("article.title")} back="/news" actions={<StaffMenu article={article} />}>
      <article className="flex flex-col gap-4">
        <ArticlePicture
          url={article.imageUrl}
          className="sticker aspect-video w-full rounded-card bg-surface-3 object-cover"
          lazy={false}
        />
        <div className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 text-sm text-ink-soft">
          {article.source && (
            <Link
              to="/news"
              search={{ source: article.source.id }}
              className="inline-flex min-w-0 items-center gap-1.5 font-bold text-ink hover:underline"
            >
              <SourceIcon source={article.source} className="size-5" />
              <span className="truncate">{article.source.name}</span>
            </Link>
          )}
          <span aria-hidden>·</span>
          <PublishedAt at={article.publishedAt} />
        </div>
        <h2 className="text-2xl leading-snug font-black break-words">{article.title}</h2>
        {article.randomPick && (
          <p className="flex items-start gap-2.5 rounded-2xl border-2 border-ink bg-sun p-3 text-sm leading-relaxed font-bold text-[#17122b]">
            <Dices className="mt-0.5 size-5 shrink-0" />
            {t("random.explain")}
          </p>
        )}
        {article.summary && (
          <p className="text-[15px] leading-relaxed break-words whitespace-pre-line text-ink-soft">
            {article.summary}
          </p>
        )}

        {url && (
          <a
            href={url}
            target="_blank"
            rel="noopener noreferrer"
            className="pressable sticker flex h-14 items-center justify-center gap-2 rounded-full bg-brand px-6 text-lg font-black text-on-brand"
          >
            <BookOpen className="size-5 shrink-0" />
            <span className="truncate">{t("article.read")}</span>
            <ExternalLink className="size-4 shrink-0 opacity-80" />
          </a>
        )}
        {url && (
          <p className="-mt-2 text-center text-xs text-ink-faint">
            {t("article.opensAt", { host: hostOf(url) })}
          </p>
        )}

        <section aria-label={t("feedback.title")} className="rounded-card bg-surface p-4">
          <h3 className="font-black">{t("feedback.title")}</h3>
          <p className="mt-0.5 mb-3 text-sm text-ink-soft">{t("feedback.lead")}</p>
          <FeedbackButtons article={article} />
          {article.feedback !== feed.NewsFeedback.UNSPECIFIED && (
            <p role="status" className="mt-3 text-sm font-bold text-brand">
              {t(
                article.feedback === feed.NewsFeedback.MORE
                  ? "feedback.thanksMore"
                  : "feedback.thanksLess",
              )}
            </p>
          )}
        </section>

        {article.tags.length > 0 && (
          <ul aria-label={t("article.tags")} className="flex flex-wrap gap-2">
            {article.tags.map((tag) => (
              <li key={tag} className="max-w-full">
                <TagLink tag={tag} className="h-8 px-3 text-sm" />
              </li>
            ))}
          </ul>
        )}

        <div className="flex flex-wrap items-center gap-x-4 gap-y-2 text-sm text-ink-soft">
          <span className="inline-flex items-center gap-1.5">
            <BookOpen className="size-4" />
            {t("article.readers", { count: Number(article.clickCount) })}
          </span>
          {article.likeCount > 0n && (
            <span className="inline-flex items-center gap-1.5">
              <Heart className="size-4" />
              {Number(article.likeCount).toLocaleString()}
            </span>
          )}
          {article.commentCount > 0n && (
            <span className="inline-flex items-center gap-1.5">
              <MessageCircle className="size-4" />
              {Number(article.commentCount).toLocaleString()}
            </span>
          )}
          <Button
            variant="ghost"
            size="sm"
            className="ml-auto"
            icon={<Share2 className="size-4" />}
            onClick={share}
          >
            {t("article.share")}
          </Button>
        </div>
      </article>
      <Related article={article} />
    </Page>
  );
}

/** More articles carrying the article's first tag. */
function Related({ article }: { article: feed.NewsArticle }) {
  const { t } = useTranslation("news");
  const tag = article.tags[0];
  const related = useQuery({
    queryKey: newsKeys.related(tag ?? ""),
    queryFn: () => newsClient().listNews({ tag, page: { limit: 6 } }),
    enabled: Boolean(tag),
    staleTime: 60_000,
  });
  const others = (related.data?.articles ?? [])
    .filter((candidate) => candidate.id !== article.id)
    .slice(0, 4);
  if (others.length === 0) return null;
  return (
    <section aria-label={t("article.related")} className="mt-8">
      <h3 className="mb-2 px-1 text-lg font-black">{t("article.related")}</h3>
      <ul className="flex flex-col gap-2">
        {others.map((other) => (
          <li key={other.id}>
            <Link
              to="/news/$articleId"
              params={{ articleId: other.id }}
              className="flex min-w-0 items-center gap-3 rounded-2xl border-2 border-line bg-surface p-3 transition-colors hover:border-ink-faint"
            >
              <span className="flex min-w-0 flex-1 flex-col gap-1">
                <span
                  className={cn(
                    "line-clamp-2 leading-snug font-black break-words",
                    other.clicked && "text-ink-soft",
                  )}
                >
                  {other.title}
                </span>
                <span className="flex min-w-0 items-center gap-1.5 text-xs text-ink-soft">
                  <SourceIcon source={other.source} />
                  <span className="truncate font-bold">{other.source?.name}</span>
                  <span aria-hidden>·</span>
                  <PublishedAt at={other.publishedAt} className="shrink-0" />
                </span>
              </span>
              <ArticlePicture
                url={other.imageUrl}
                className="size-16 shrink-0 rounded-xl bg-surface-3 object-cover"
              />
            </Link>
          </li>
        ))}
      </ul>
    </section>
  );
}

/** Moderators and admins can take an article out of every list from its page. */
function StaffMenu({ article }: { article: feed.NewsArticle }) {
  const { t } = useTranslation("news");
  const roles = useMe()?.roles ?? [];
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const [confirming, setConfirming] = useState(false);
  const remove = useMutation({
    mutationFn: () => newsAdminClient().removeNewsArticle({ articleId: article.id }),
    onSuccess: () => {
      dropArticle(queryClient, article.id);
      toast.success(t("staff.articleRemoved"));
      void navigate({ to: "/news" });
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "news" })),
    onSettled: () => setConfirming(false),
  });
  if (!roles.includes("admin") && !roles.includes("moderator")) return null;
  return (
    <>
      <Menu
        trigger={
          <Button variant="ghost" size="icon-sm" aria-label={t("staff.menu")}>
            <MoreHorizontal className="size-5" />
          </Button>
        }
        actions={[
          {
            label: t("staff.removeArticle"),
            icon: <Trash2 />,
            danger: true,
            onSelect: () => setConfirming(true),
          },
        ]}
      />
      <ConfirmDialog
        open={confirming}
        onOpenChange={setConfirming}
        title={t("staff.removeArticleTitle")}
        description={t("staff.removeArticleBody")}
        confirmLabel={t("staff.removeArticle")}
        danger
        loading={remove.isPending}
        onConfirm={() => remove.mutate()}
      />
    </>
  );
}
