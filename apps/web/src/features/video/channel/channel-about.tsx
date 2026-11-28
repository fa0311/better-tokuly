import { toDate, type video } from "@tokuly/api";
import { CalendarDays, ExternalLink, Eye, Film, Users } from "lucide-react";
import type { ReactNode } from "react";
import { useTranslation } from "#/lib/use-translation";
import { formatCount } from "../format";

/** The channel's about tab: what it says about itself, its links and its numbers. */
export function ChannelAbout({ channel }: { channel: video.Channel }) {
  const { t, i18n } = useTranslation("video");
  const created = toDate(channel.createdAt);
  return (
    <div className="grid gap-5 lg:grid-cols-[minmax(0,1fr)_20rem]">
      <section className="sticker rounded-card bg-surface p-5">
        <h2 className="text-lg font-black">{t("channel.about.description")}</h2>
        {channel.description ? (
          <p className="mt-2 leading-relaxed break-words whitespace-pre-wrap">
            {channel.description}
          </p>
        ) : (
          <p className="mt-2 text-ink-faint">{t("channel.about.noDescription")}</p>
        )}
        {channel.links.length > 0 && (
          <>
            <h2 className="mt-6 text-lg font-black">{t("channel.about.links")}</h2>
            <ul className="mt-2 flex flex-col gap-1">
              {channel.links.map((link) => (
                <li key={link.url}>
                  <a
                    href={link.url}
                    target="_blank"
                    rel="noopener noreferrer nofollow ugc"
                    className="flex items-center gap-3 rounded-2xl px-3 py-2 hover:bg-surface-2"
                  >
                    <ExternalLink className="size-4 shrink-0 text-brand" />
                    <span className="min-w-0 flex-1">
                      <span className="block truncate font-bold">{link.title || link.url}</span>
                      <span className="block truncate text-sm text-ink-soft">{link.url}</span>
                    </span>
                  </a>
                </li>
              ))}
            </ul>
          </>
        )}
      </section>
      <section className="sticker self-start rounded-card bg-surface p-5">
        <h2 className="text-lg font-black">{t("channel.about.stats")}</h2>
        <dl className="mt-3 flex flex-col gap-3 text-sm">
          <Stat
            icon={<Users className="size-4" />}
            label={t("channel.about.subscribers")}
            value={formatCount(channel.summary?.subscriberCount ?? 0n)}
          />
          <Stat
            icon={<Film className="size-4" />}
            label={t("channel.about.videos")}
            value={formatCount(channel.videoCount)}
          />
          <Stat
            icon={<Eye className="size-4" />}
            label={t("channel.about.views")}
            value={formatCount(channel.viewCount)}
          />
          {created && (
            <Stat
              icon={<CalendarDays className="size-4" />}
              label={t("channel.about.joined")}
              value={created.toLocaleDateString(i18n.language, {
                year: "numeric",
                month: "long",
                day: "numeric",
              })}
            />
          )}
        </dl>
      </section>
    </div>
  );
}

function Stat({ icon, label, value }: { icon: ReactNode; label: string; value: string }) {
  return (
    <div className="flex items-center gap-3">
      <span className="grid size-9 shrink-0 place-items-center rounded-xl bg-surface-2 text-ink-soft">
        {icon}
      </span>
      <dt className="flex-1 font-bold text-ink-soft">{label}</dt>
      <dd className="font-black tabular-nums">{value}</dd>
    </div>
  );
}
