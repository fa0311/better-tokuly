import { identity } from "@tokuly/api";
import { BadgeCheck, ExternalLink, Music } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Card, Chip } from "#/components/ui/card";
import { EmojiText } from "#/features/emoji";
import type { ProfileSectionProps } from "#/seams/profile";

/** Only http(s) links leave the app from a profile. */
const safeUrl = (url: string) => (/^https?:\/\//i.test(url) ? url : undefined);

/**
 * The プロフ帳 as the viewer may see it: what the server sends is already cut
 * to the viewer's audience, so this shows whatever is there.
 */
export function ProfileBook({ profile }: ProfileSectionProps) {
  const { t, i18n } = useTranslation("profileBook");
  const extras = profile.extras;
  if (!extras) return null;
  const en = i18n.language.startsWith("en");
  const facts: { label: string; value: string }[] = [];
  if (extras.mbti) facts.push({ label: t("about.mbti"), value: extras.mbti });
  if (extras.bloodType)
    facts.push({
      label: t("about.bloodType"),
      value: t(`blood.${identity.BloodType[extras.bloodType]}`),
    });
  if (extras.zodiac)
    facts.push({ label: t("about.zodiac"), value: t(`zodiac.${identity.Zodiac[extras.zodiac]}`) });
  if (profile.birthday?.month)
    facts.push({
      label: t("view.birthday"),
      value: profile.birthday.year
        ? t("view.birthdayFull", {
            year: profile.birthday.year,
            month: profile.birthday.month,
            day: profile.birthday.day,
          })
        : t("view.birthdayDay", { month: profile.birthday.month, day: profile.birthday.day }),
    });
  const accent = extras.theme?.color || undefined;
  const nothing =
    facts.length === 0 &&
    !extras.longBio &&
    extras.oshi.length === 0 &&
    extras.tags.length === 0 &&
    extras.links.length === 0 &&
    extras.qaCards.length === 0 &&
    !extras.pinnedSong?.title;
  if (nothing) return null;

  return (
    <section
      className="mt-5 flex flex-col gap-4"
      aria-label={t("title")}
      data-style={identity.ProfileThemeStyle[extras.theme?.style ?? 0]}
    >
      <h2 className="px-1 text-lg font-black" style={accent ? { color: accent } : undefined}>
        📒 {t("title")}
      </h2>
      {facts.length > 0 && (
        <Card
          className="grid grid-cols-2 gap-3 sm:grid-cols-4"
          style={accent ? { borderColor: accent } : undefined}
        >
          {facts.map((fact) => (
            <div key={fact.label}>
              <p className="text-xs font-bold text-ink-faint">{fact.label}</p>
              <p className="font-black">{fact.value}</p>
            </div>
          ))}
        </Card>
      )}
      {extras.longBio && (
        <Card>
          <p className="leading-relaxed whitespace-pre-wrap">
            <EmojiText text={extras.longBio} />
          </p>
        </Card>
      )}
      {extras.oshi.length > 0 && (
        <div>
          <h3 className="px-1 text-sm font-black text-ink-soft">{t("oshi.title")}</h3>
          <ul className="mt-2 flex flex-wrap gap-3">
            {extras.oshi.map((oshi) => {
              const href = safeUrl(oshi.url);
              const body = (
                <>
                  <span className="grid size-12 place-items-center overflow-hidden rounded-xl border-2 border-ink bg-pink text-xl text-[#17122b]">
                    {oshi.image?.url ? (
                      <img src={oshi.image.url} alt="" className="size-full object-cover" />
                    ) : (
                      oshi.emoji || "💖"
                    )}
                  </span>
                  <span className="max-w-28 truncate text-sm font-black">{oshi.name}</span>
                </>
              );
              return (
                <li key={oshi.id}>
                  {href ? (
                    <a
                      href={href}
                      target="_blank"
                      rel="noreferrer noopener"
                      className="pressable flex flex-col items-center gap-1"
                    >
                      {body}
                    </a>
                  ) : (
                    <span className="flex flex-col items-center gap-1">{body}</span>
                  )}
                </li>
              );
            })}
          </ul>
        </div>
      )}
      {extras.tags.length > 0 && (
        <ul className="flex flex-wrap gap-1.5" aria-label={t("tags.title")}>
          {extras.tags.map((tag) => (
            <li key={`${tag.category}-${tag.name}`}>
              <Chip>
                <span className="text-ink-faint">#</span>
                {tag.name}
              </Chip>
            </li>
          ))}
        </ul>
      )}
      {extras.links.length > 0 && (
        <ul className="flex flex-col gap-1">
          {extras.links.map((link) => {
            const href = safeUrl(link.url);
            return (
              <li key={link.id}>
                <a
                  href={href}
                  target="_blank"
                  rel={link.verified ? "me noreferrer noopener" : "nofollow noreferrer noopener"}
                  className="inline-flex items-center gap-1.5 font-bold text-brand underline-offset-4 hover:underline"
                >
                  <ExternalLink className="size-4" />
                  {link.title}
                  {link.verified && (
                    <BadgeCheck className="size-4 text-success" aria-label={t("links.verified")} />
                  )}
                </a>
              </li>
            );
          })}
        </ul>
      )}
      {extras.pinnedSong?.title && (
        <a
          href={safeUrl(extras.pinnedSong.trackUrl)}
          target="_blank"
          rel="noreferrer noopener"
          className="sticker flex items-center gap-3 rounded-card bg-surface p-3"
        >
          {extras.pinnedSong.artworkUrl ? (
            <img
              src={extras.pinnedSong.artworkUrl}
              alt=""
              className="size-14 rounded-xl object-cover"
            />
          ) : (
            <span className="grid size-14 place-items-center rounded-xl bg-mint text-[#17122b]">
              <Music className="size-6" />
            </span>
          )}
          <span className="min-w-0 flex-1">
            <span className="block text-xs font-black text-success">{t("song.title")}</span>
            <span className="block truncate font-black">{extras.pinnedSong.title}</span>
            <span className="block truncate text-sm text-ink-soft">{extras.pinnedSong.artist}</span>
          </span>
        </a>
      )}
      {extras.qaCards.length > 0 && (
        <ul className="grid gap-3 sm:grid-cols-2" aria-label={t("cards.title")}>
          {extras.qaCards.map((card) => (
            <li
              key={card.id}
              className="sticker-sm rounded-card bg-surface p-4"
              data-testid="qa-card"
            >
              <p className="text-xs font-black text-ink-soft">
                {en ? card.question?.en : card.question?.ja}
              </p>
              <p className="mt-1 font-bold whitespace-pre-wrap">
                <EmojiText text={card.answer} />
              </p>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
