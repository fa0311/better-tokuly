import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { ChevronLeft } from "lucide-react";
import { useTranslation } from "react-i18next";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { errorMessage } from "#/lib/errors";
import { announcementClient } from "./api";
import { AnnouncementArticle } from "./parts";

/** `/announcements/:announcementId`: one announcement in full. */
export function AnnouncementScreen({ announcementId }: { announcementId: string }) {
  const { t, i18n } = useTranslation("announcements");
  const detail = useQuery({
    queryKey: ["admin", "announcements", "published", i18n.language, "detail", announcementId],
    queryFn: () =>
      announcementClient().getAnnouncement({ id: announcementId, locale: i18n.language }),
  });
  const announcement = detail.data?.announcement;

  return (
    <Page title={t("title")} back="/announcements">
      {detail.isPending ? (
        <div className="flex flex-col gap-4" aria-busy>
          <Skeleton className="h-48" />
          <Skeleton className="h-72" />
        </div>
      ) : !announcement ? (
        <EmptyState
          emoji="🧭"
          title={detail.isError ? errorMessage(detail.error) : t("errors.ANNOUNCEMENT_NOT_FOUND")}
          action={
            <Link
              to="/announcements"
              className="pressable sticker inline-flex h-11 items-center rounded-full bg-brand px-5 font-bold text-on-brand"
            >
              {t("backToList")}
            </Link>
          }
        />
      ) : (
        <AnnouncementArticle
          announcement={announcement}
          footer={
            <Link
              to="/announcements"
              className="mt-8 inline-flex items-center gap-1 text-sm font-bold text-brand hover:underline"
            >
              <ChevronLeft className="size-4" />
              {t("backToList")}
            </Link>
          }
        />
      )}
    </Page>
  );
}
