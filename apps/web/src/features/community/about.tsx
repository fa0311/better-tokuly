import { Link } from "@tanstack/react-router";
import type { community } from "@tokuly/api";
import { VISIBILITY_KEYS } from "@tokuly/core/community";
import { Avatar } from "#/components/ui/avatar";
import { EmojiText } from "#/features/emoji";
import { useTranslation } from "#/lib/use-translation";
import { NS } from "./api";
import { useWhen } from "./parts";

/** The rules of a community, numbered. */
export function Rules({ community: c }: { community: community.Community }) {
  const { t } = useTranslation(NS);
  if (c.rules.length === 0) return null;
  return (
    <section className="rounded-card bg-surface p-4">
      <h3 className="font-black">{t("about.rules")}</h3>
      <ol className="mt-2 flex flex-col gap-3">
        {c.rules.map((rule, index) => (
          <li key={index} className="flex gap-3">
            <span className="grid size-7 shrink-0 place-items-center rounded-full bg-brand-soft text-sm font-black text-brand-strong">
              {index + 1}
            </span>
            <div className="min-w-0">
              <p className="break-words font-bold">
                <EmojiText text={rule.title} />
              </p>
              {rule.body && (
                <p className="whitespace-pre-wrap break-words text-sm leading-relaxed text-ink-soft">
                  <EmojiText text={rule.body} />
                </p>
              )}
            </div>
          </li>
        ))}
      </ol>
    </section>
  );
}

/** Who runs it, since when, and how one gets in. */
export function Facts({ community: c }: { community: community.Community }) {
  const { t } = useTranslation(NS);
  const when = useWhen();
  return (
    <section className="rounded-card bg-surface p-4">
      <h3 className="font-black">{t("about.title")}</h3>
      <dl className="mt-2 flex flex-col gap-2.5 text-sm">
        {c.owner && (
          <div className="flex items-center gap-2">
            <dt className="w-20 shrink-0 text-ink-soft">{t("role.owner")}</dt>
            <dd className="min-w-0">
              <Link
                to="/u/$handle"
                params={{ handle: c.owner.handle }}
                className="flex items-center gap-2 font-bold"
              >
                <Avatar user={c.owner} size="xs" />
                <span className="truncate">
                  <EmojiText text={c.owner.displayName || c.owner.handle} />
                </span>
              </Link>
            </dd>
          </div>
        )}
        <div className="flex gap-2">
          <dt className="w-20 shrink-0 text-ink-soft">{t("about.visibility")}</dt>
          <dd className="min-w-0">
            <p className="font-bold">{t(`visibility.${VISIBILITY_KEYS[c.visibility]}`)}</p>
            <p className="text-ink-soft">{t(`visibility.${VISIBILITY_KEYS[c.visibility]}Hint`)}</p>
          </dd>
        </div>
        <div className="flex gap-2">
          <dt className="w-20 shrink-0 text-ink-soft">{t("about.members")}</dt>
          <dd className="font-bold">
            {t("about.memberOf", {
              count: c.memberCount.toLocaleString(),
              max: c.maxMembers.toLocaleString(),
            })}
          </dd>
        </div>
        <div className="flex gap-2">
          <dt className="w-20 shrink-0 text-ink-soft">{t("about.threads")}</dt>
          <dd className="font-bold">{c.threadCount.toLocaleString()}</dd>
        </div>
        {c.createdAt && (
          <div className="flex gap-2">
            <dt className="w-20 shrink-0 text-ink-soft">{t("about.created")}</dt>
            <dd className="font-bold">
              {new Intl.DateTimeFormat(undefined, { dateStyle: "medium" }).format(
                new Date(Number(c.createdAt.seconds) * 1000),
              )}
            </dd>
          </div>
        )}
        {c.lastActivityAt && (
          <div className="flex gap-2">
            <dt className="w-20 shrink-0 text-ink-soft">{t("about.active")}</dt>
            <dd className="font-bold">{when(Number(c.lastActivityAt.seconds) * 1000)}</dd>
          </div>
        )}
      </dl>
    </section>
  );
}
