import { Link } from "@tanstack/react-router";
import { EmojiText } from "#/features/emoji";
import { useTranslation } from "#/lib/use-translation";
import type { ProfileSection, ProfileSectionProps } from "#/seams/profile";
import { NS, useUserCommunities } from "./api";
import { CommunityIcon } from "./parts";

/** The public communities a person chose to show on their profile, as a row of chips. */
function ProfileCommunities({ user }: ProfileSectionProps) {
  const { t } = useTranslation(NS);
  const communities = useUserCommunities(user.id);
  if (!communities.items?.length) return null;
  return (
    <section className="mt-5" aria-label={t("profile.title")}>
      <h3 className="px-1 pb-2 text-sm font-black text-ink-soft">{t("profile.title")}</h3>
      <div className="scrollbar-none -mx-4 flex gap-2 overflow-x-auto px-4 pb-1">
        {communities.items.map((c) => (
          <Link
            key={c.id}
            to="/g/$handle"
            params={{ handle: c.handle }}
            className="sticker-sm pressable flex shrink-0 items-center gap-2 rounded-2xl bg-surface py-1.5 pr-3.5 pl-1.5"
          >
            <CommunityIcon community={c} size="sm" className="border-0 shadow-none" />
            <span className="max-w-40 truncate text-sm font-bold">
              <EmojiText text={c.name} />
            </span>
          </Link>
        ))}
      </div>
    </section>
  );
}

export default [
  { id: "community", slot: "card", order: 60, Component: ProfileCommunities },
] satisfies ProfileSection[];
