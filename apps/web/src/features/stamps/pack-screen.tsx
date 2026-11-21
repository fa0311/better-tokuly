import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { stamp, toDate } from "@tokuly/api";
import { BadgeCheck, Check, Download, Pencil, Share2 } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { Chip } from "#/components/ui/card";
import { CoinIcon } from "#/components/ui/coins";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { formatCount, stampKeys, storeClient, useInstallPack, usePack, useUninstallPack } from "./api";
import { AnimatedMark, PackCard, PackCover, SectionHeading, StampGrid, StampTile, StatusChip, TagLink } from "./parts";
import { PurchaseDialog } from "./purchase";
import { StampSheet } from "./stamp-sheet";

/** A pack in the store: what it is, who made it, every stamp, and the way to get it. */
export function PackScreen({ packId }: { packId: string }) {
  const { t, i18n } = useTranslation("stamps");
  const { data: pack, isPending, isError } = usePack(packId);
  const [openStamp, setOpenStamp] = useState<string>();
  const summary = pack?.summary;

  if (isPending) {
    return (
      <Page title=" " back="/stamps" wide>
        <div className="grid gap-5 md:grid-cols-[20rem_1fr]">
          <Skeleton className="h-72" />
          <Skeleton className="h-96" />
        </div>
      </Page>
    );
  }
  if (isError || !pack || !summary) {
    return (
      <Page title={t("title")} back="/stamps">
        <EmptyState
          emoji="🫥"
          title={t("pack.notFound")}
          body={t("pack.notFoundBody")}
          action={
            <Link to="/stamps" className="pressable sticker inline-flex h-11 items-center rounded-full bg-brand px-5 font-bold text-on-brand">
              {t("pack.toStore")}
            </Link>
          }
        />
      </Page>
    );
  }

  const published = toDate(summary.publishedAt);
  const share = async () => {
    const url = `${window.location.origin}/stamps/${summary.shortId}`;
    if (navigator.share) {
      await navigator.share({ title: summary.name, url }).catch(() => {});
    } else {
      await navigator.clipboard.writeText(url);
      toast.success(t("common:actions.copied"));
    }
  };

  return (
    <Page
      title={summary.name}
      back="/stamps"
      wide
      actions={
        <Button size="icon-sm" variant="ghost" aria-label={t("common:actions.share")} onClick={share}>
          <Share2 className="size-5" />
        </Button>
      }
    >
      <div className="grid items-start gap-5 md:grid-cols-[20rem_1fr]">
        <section className="sticker flex flex-col gap-4 rounded-card bg-surface p-4 md:sticky md:top-20">
          <div className="flex items-center gap-4 md:flex-col md:items-stretch">
            <PackCover pack={summary} className="size-28 shrink-0 md:size-auto" />
            <div className="min-w-0 flex-1">
              <h2 className="text-xl leading-tight font-black break-words">{summary.name}</h2>
              <Maker summary={summary} />
              <div className="mt-2 flex flex-wrap items-center gap-1.5">
                {summary.status !== stamp.PackStatus.PUBLISHED && <StatusChip status={summary.status} />}
                <Chip>{t("stampCount", { count: summary.stampCount })}</Chip>
                {summary.animated && (
                  <Chip>
                    <AnimatedMark className="size-4 border-0 bg-transparent" />
                    {t("animated")}
                  </Chip>
                )}
              </div>
            </div>
          </div>

          <PackActions summary={summary} />

          {pack.description && <p className="text-[15px] leading-relaxed break-words whitespace-pre-wrap">{pack.description}</p>}
          {summary.tags.length > 0 && (
            <div className="flex flex-wrap gap-2">
              {summary.tags.map((tag) => (
                <TagLink key={tag} tag={tag} />
              ))}
            </div>
          )}
          <dl className="grid grid-cols-2 gap-2 border-t-2 border-line pt-3 text-center">
            <div>
              <dd className="text-lg font-black tabular-nums">{formatCount(summary.installCount)}</dd>
              <dt className="text-xs font-bold text-ink-faint">{t("pack.installs")}</dt>
            </div>
            <div>
              <dd className="text-lg font-black tabular-nums">{formatCount(summary.sendCount)}</dd>
              <dt className="text-xs font-bold text-ink-faint">{t("pack.sends")}</dt>
            </div>
          </dl>
          {published && <p className="text-center text-xs text-ink-faint">{t("pack.publishedOn", { date: published.toLocaleDateString(i18n.language) })}</p>}
        </section>

        <div className="flex min-w-0 flex-col gap-6">
          <section aria-label={t("pack.stamps")}>
            <SectionHeading title={t("pack.stamps")} action={<span className="text-xs font-bold text-ink-faint">{t("pack.tapHint")}</span>} />
            {pack.stamps.length === 0 ? (
              <EmptyState emoji="🫙" title={t("pack.noStamps")} />
            ) : (
              <StampGrid className="sticker rounded-card bg-surface p-2 md:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6">
                {pack.stamps.map((item) => (
                  <StampTile key={item.stampRef?.id} item={item} onPress={(pressed) => setOpenStamp(pressed.stampRef?.id)} />
                ))}
              </StampGrid>
            )}
          </section>
          {summary.creator && !summary.creator.deleted && <MoreByCreator creatorId={summary.creator.id} exceptPackId={summary.id} />}
        </div>
      </div>
      <StampSheet stampId={openStamp} onClose={() => setOpenStamp(undefined)} />
    </Page>
  );
}

function Maker({ summary }: { summary: stamp.PackSummary }) {
  const { t } = useTranslation("stamps");
  if (summary.official) {
    return (
      <Link to="/stamps/browse" search={{ section: "official" }} className="mt-1 inline-flex items-center gap-1 text-sm font-bold text-brand">
        <BadgeCheck className="size-4" />
        {t("official")}
      </Link>
    );
  }
  const creator = summary.creator;
  if (!creator || creator.deleted) return <p className="mt-1 text-sm text-ink-soft">{t("deletedCreator")}</p>;
  return (
    <Link to="/stamps/creator/$userId" params={{ userId: creator.id }} className="mt-1 flex min-w-0 items-center gap-1.5 text-sm font-bold text-ink-soft hover:text-ink">
      <Avatar user={creator} size="xs" />
      <span className="truncate">{creator.displayName}</span>
    </Link>
  );
}

/** The main button of a pack page: add, buy, or what the viewer already did. */
function PackActions({ summary }: { summary: stamp.PackSummary }) {
  const { t } = useTranslation("stamps");
  const install = useInstallPack();
  const uninstall = useUninstallPack();
  const [buying, setBuying] = useState(false);
  const viewer = summary.viewer;

  return (
    <div className="flex flex-col gap-2">
      {viewer?.installed ? (
        <div className="flex items-center gap-2">
          <p className="flex h-11 flex-1 items-center justify-center gap-1.5 rounded-full bg-surface-2 font-bold text-success">
            <Check className="size-4" strokeWidth={3} />
            {t("state.installed")}
          </p>
          <Button variant="ghost" loading={uninstall.isPending} onClick={() => uninstall.mutate(summary.id)}>
            {t("pack.uninstall")}
          </Button>
        </div>
      ) : summary.price === 0 || viewer?.owned ? (
        <Button variant="lime" block icon={<Download className="size-4" />} loading={install.isPending} onClick={() => install.mutate(summary.id)}>
          {viewer?.owned ? t("pack.reinstall") : t("pack.installFree")}
        </Button>
      ) : (
        <Button block onClick={() => setBuying(true)} icon={<CoinIcon className="size-5 text-[11px]" />}>
          {t("pack.buy", { price: summary.price.toLocaleString("en-US") })}
        </Button>
      )}
      {viewer?.installed && <p className="text-center text-xs text-ink-faint">{viewer.owned ? t("pack.ownedHint") : t("pack.installedHint")}</p>}
      {viewer?.canManage && (
        <Link
          to="/stamps/studio/$packId"
          params={{ packId: summary.shortId || summary.id }}
          className="pressable sticker inline-flex h-11 items-center justify-center gap-2 rounded-full bg-surface font-bold"
        >
          <Pencil className="size-4" />
          {t("pack.editInStudio")}
        </Link>
      )}
      <PurchaseDialog pack={summary} open={buying} onOpenChange={setBuying} />
    </div>
  );
}

function MoreByCreator({ creatorId, exceptPackId }: { creatorId: string; exceptPackId: string }) {
  const { t } = useTranslation("stamps");
  const { data } = useQuery({
    queryKey: [...stampKeys.creator(creatorId), "preview"],
    queryFn: () => storeClient().listCreatorPacks({ userId: creatorId, page: { limit: 7 } }),
  });
  const others = data?.packs.filter((pack) => pack.id !== exceptPackId).slice(0, 6) ?? [];
  if (others.length === 0) return null;
  return (
    <section aria-label={t("pack.moreByCreator")}>
      <SectionHeading
        title={t("pack.moreByCreator")}
        action={
          <Link to="/stamps/creator/$userId" params={{ userId: creatorId }} className="text-sm font-bold text-brand">
            {t("store.more")}
          </Link>
        }
      />
      <div className="grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-4 xl:grid-cols-6">
        {others.map((pack) => (
          <PackCard key={pack.id} pack={pack} />
        ))}
      </div>
    </section>
  );
}
