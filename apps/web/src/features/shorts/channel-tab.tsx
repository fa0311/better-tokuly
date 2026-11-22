import { Link } from "@tanstack/react-router";
import { shorts, type video } from "@tokuly/api";
import { Plus } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { EmptyState } from "#/components/ui/page";
import type { ChannelTab } from "#/features/video";
import i18n from "#/lib/i18n";
import { ShortList } from "./short-tile";
import { ChoiceChips } from "./sort-chips";

const { ChannelShortSort } = shorts;

/** The channel page's shorts: pinned ones first, then the newest; or the most viewed. */
function ChannelShorts({ channel }: { channel: video.Channel }) {
  const { t } = useTranslation("shorts");
  const [sort, setSort] = useState<shorts.ChannelShortSort>(ChannelShortSort.LATEST);
  const handle = channel.summary?.handle ?? "";
  const own = channel.viewer?.isOwner ?? false;
  return (
    <div>
      <div className="mb-4 flex flex-wrap items-center justify-between gap-2">
        <ChoiceChips
          label={t("channel.sortLabel")}
          value={sort}
          onChange={(next) => setSort(next)}
          options={[
            { value: ChannelShortSort.LATEST, label: t("channel.sort.latest") },
            { value: ChannelShortSort.POPULAR, label: t("channel.sort.popular") },
          ]}
        />
        {own && (
          <Link
            to="/shorts/new"
            className="pressable sticker-sm inline-flex h-9 items-center gap-1.5 rounded-full bg-brand px-4 text-sm font-bold text-on-brand"
          >
            <Plus className="size-4" />
            {t("channel.post")}
          </Link>
        )}
      </div>
      <ShortList
        context={{ kind: "channel", key: handle }}
        options={{ sort }}
        empty={
          <EmptyState
            emoji="⚡"
            title={own ? t("channel.emptyOwn") : t("channel.empty")}
            body={own ? t("channel.emptyOwnBody") : undefined}
          />
        }
      />
    </div>
  );
}

export default {
  id: "shorts",
  order: 20,
  label: () => i18n.t("shorts:channel.tab"),
  Component: ChannelShorts,
} satisfies ChannelTab;
