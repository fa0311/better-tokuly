import { common } from "@tokuly/api";
import { latenessText, reactionBlocker } from "@tokuly/core/moments";
import { Flag } from "lucide-react";
import { lazy, Suspense, useState } from "react";
import { useTranslation } from "react-i18next";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { EmojiText } from "#/features/emoji";
import { ReportDialog } from "#/features/engagement";
import { useMoment, useReactions } from "./api";
import { MomentPair } from "./pair";
import { ReactionFace, ReactionPicker } from "./reactions";

const StaticMapPreview = lazy(() =>
  import("#/features/map").then((module) => ({ default: module.StaticMapPreview })),
);

/** One moment, as the caller may see it, with everyone who reacted. */
export function MomentScreen({ momentId }: { momentId: string }) {
  const { t, i18n } = useTranslation("moments");
  const { data: moment, isPending, isError } = useMoment(momentId);
  const reactions = useReactions(momentId, Boolean(moment && !moment.locked));
  const [reporting, setReporting] = useState(false);
  const list = reactions.data?.pages.flatMap((page) => page.reactions) ?? [];

  return (
    <Page
      title={moment?.author ? t("card.by", { name: moment.author.displayName }) : t("title")}
      back={{ fallback: "/moments" }}
      actions={
        moment &&
        !moment.mine && (
          <Button
            variant="ghost"
            size="icon-sm"
            aria-label={t("card.report")}
            onClick={() => setReporting(true)}
          >
            <Flag className="size-4" />
          </Button>
        )
      }
    >
      {isPending ? (
        <Skeleton className="aspect-[3/4] w-full" />
      ) : isError || !moment ? (
        <EmptyState emoji="🙈" title={t("notFound")} />
      ) : (
        <div className="flex flex-col gap-4" data-testid="moment-detail">
          <div className="flex items-center gap-2.5">
            <Avatar user={moment.author} size="md" />
            <p className="min-w-0 flex-1 truncate font-black">
              <EmojiText text={moment.author?.displayName ?? ""} />
            </p>
            <span className="rounded-full bg-surface-2 px-2.5 py-0.5 text-xs font-black">
              {latenessText(i18n, moment.lateSeconds)}
            </span>
          </div>
          <MomentPair
            moment={moment}
            width={640}
            locked={<p className="font-black">{t("locked.title")}</p>}
          />
          {moment.caption && (
            <p className="text-lg font-bold">
              <EmojiText text={moment.caption} />
            </p>
          )}
          {moment.location && (
            <Suspense fallback={<Skeleton className="aspect-video" />}>
              <StaticMapPreview
                lat={moment.location.lat}
                lng={moment.location.lng}
                label={t("card.place")}
                className="aspect-video w-full rounded-2xl"
              />
            </Suspense>
          )}
          {reactionBlocker(moment) === null && <ReactionPicker moment={moment} />}
          {!moment.locked && (
            <section>
              <h2 className="mb-2 text-base font-black">
                {t("card.reactions", { count: moment.reactionCount })}
              </h2>
              <ul className="flex flex-col gap-2" data-testid="moment-reaction-list">
                {list.map((reaction) => (
                  <li key={reaction.id} className="flex items-center gap-3">
                    <ReactionFace reaction={reaction} size={48} />
                    <span className="min-w-0 flex-1 truncate font-bold">
                      <EmojiText text={reaction.user?.displayName ?? ""} />
                    </span>
                  </li>
                ))}
              </ul>
              <LoadMore query={reactions} />
            </section>
          )}
          <ReportDialog
            subject={{ type: common.SubjectType.MOMENT, id: moment.id }}
            open={reporting}
            onOpenChange={setReporting}
            label={t("card.by", { name: moment.author?.displayName ?? "" })}
          />
        </div>
      )}
    </Page>
  );
}
