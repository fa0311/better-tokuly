import { Link } from "@tanstack/react-router";
import { Plus } from "lucide-react";
import { useState } from "react";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { useTranslation } from "#/lib/use-translation";
import { ChannelAnalytics, type RangeDays, RangePicker } from "./analytics";
import { ChannelPicker, selectStudioChannel, useStudioChannel } from "./shared";

/** `/studio/analytics`: how a channel is doing — pick the channel and the period, and everything below follows. */
export function AnalyticsScreen() {
  const { t } = useTranslation("video");
  const { query, channels, channel } = useStudioChannel();
  const [days, setDays] = useState<RangeDays>(28);
  const channelId = channel?.summary?.id;

  return (
    <Page title={t("studio.analytics.title")} back="/studio" wide>
      {query.isPending ? (
        <Skeleton className="h-96 rounded-card" />
      ) : !channelId ? (
        <EmptyState
          emoji="📊"
          title={t("studio.analytics.needChannel")}
          action={
            <Link
              to="/studio/channels/new"
              className="pressable sticker inline-flex h-11 items-center gap-2 rounded-full bg-brand px-5 font-bold text-on-brand"
            >
              <Plus className="size-4" />
              {t("studio.welcome.action")}
            </Link>
          }
        />
      ) : (
        <div className="flex flex-col gap-4">
          <div className="flex flex-col gap-3">
            <ChannelPicker
              channels={channels}
              selectedId={channelId}
              onSelect={selectStudioChannel}
            />
            <RangePicker days={days} onChange={setDays} />
          </div>
          <ChannelAnalytics key={channelId} channelId={channelId} days={days} />
        </div>
      )}
    </Page>
  );
}
