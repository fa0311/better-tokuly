import { useQuery } from "@tanstack/react-query";
import { MapPin } from "lucide-react";
import { Chip } from "#/components/ui/card";
import { Skeleton } from "#/components/ui/page";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { useTranslation } from "#/lib/use-translation";
import { type dev, localized, scopesQuery } from "./api";
import { Notice } from "./parts";
import {
  groupScopes,
  LOCATION_DOMAIN,
  levelOf,
  type ScopeGroup,
  type ScopeLevel,
  withLevel,
  writesWithoutSpending,
} from "./scopes";

/**
 * Choosing what a token or an application may do: one row per domain of the
 * scope catalogue (nothing, read, or read and write), location first and on
 * its own. The wording comes from the catalogue.
 */
export function ScopePicker({
  value,
  onChange,
  offered,
}: {
  value: string[];
  onChange(scopes: string[]): void;
  /** Leaves scopes out of the choice (a bot's token cannot hold every scope). */
  offered?(scope: dev.ScopeInfo): boolean;
}) {
  const { t } = useTranslation("dev");
  const catalogue = useQuery(scopesQuery);
  if (catalogue.isPending) return <Skeleton className="h-64" />;
  if (catalogue.isError)
    return <Notice tone="danger">{errorMessage(catalogue.error, { namespace: "dev" })}</Notice>;

  const groups = groupScopes(catalogue.data.filter((scope) => offered?.(scope) ?? true));
  const location = groups.find((group) => group.domain === LOCATION_DOMAIN);
  const others = groups.filter((group) => group !== location);
  const set = (domain: string, level: ScopeLevel) => onChange(withLevel(value, domain, level));

  return (
    <div className="flex flex-col gap-3">
      {location && (
        <div className="rounded-2xl border-2 border-ink bg-sun/25 p-3">
          <p className="flex items-center gap-1.5 text-sm font-black">
            <MapPin className="size-4" />
            {t("scopes.locationTitle")}
          </p>
          <p className="mt-1 text-xs font-bold leading-relaxed text-ink-soft">
            {t("scopes.locationWarning")}
          </p>
          <ScopeRow group={location} level={levelOf(value, location.domain)} onChange={set} bare />
        </div>
      )}
      <ul className="flex flex-col divide-y-2 divide-line rounded-2xl border-2 border-line">
        {others.map((group) => (
          <li key={group.domain} className="px-3">
            <ScopeRow group={group} level={levelOf(value, group.domain)} onChange={set} />
          </li>
        ))}
      </ul>
      {writesWithoutSpending(value) && groups.some((group) => group.domain === "wallet") && (
        <Notice>{t("scopes.spendHint")}</Notice>
      )}
    </div>
  );
}

function ScopeRow({
  group,
  level,
  onChange,
  bare,
}: {
  group: ScopeGroup<dev.ScopeInfo>;
  level: ScopeLevel;
  onChange(domain: string, level: ScopeLevel): void;
  /** Inside a box of its own: no "sensitive" label (the box says why). */
  bare?: boolean;
}) {
  const { t } = useTranslation("dev");
  const levels: ScopeLevel[] = [
    "none",
    ...(group.read ? (["read"] as const) : []),
    ...(group.write ? (["write"] as const) : []),
  ];
  // What the chosen level allows; before choosing, what reading would.
  const described = level === "write" ? group.write : (group.read ?? group.write);
  return (
    <div className="py-2.5">
      <div className="flex flex-wrap items-center gap-x-2 gap-y-1.5">
        <span className="font-mono text-sm font-bold">{group.domain}</span>
        {group.sensitive && !bare && (
          <Chip className="bg-sun text-[#17122b]">{t("scopes.sensitive")}</Chip>
        )}
        <div
          role="radiogroup"
          aria-label={group.domain}
          className="ml-auto inline-flex gap-0.5 rounded-full bg-surface-2 p-0.5"
        >
          {levels.map((option) => (
            // biome-ignore lint/a11y/useSemanticElements: a segmented control; the group handles the choice like radio buttons.
            <button
              key={option}
              type="button"
              role="radio"
              aria-checked={level === option}
              onClick={() => onChange(group.domain, option)}
              className={cn(
                "h-8 rounded-full px-3 text-xs font-black transition-colors",
                level === option
                  ? option === "none"
                    ? "bg-surface text-ink"
                    : "bg-brand text-on-brand"
                  : "text-ink-soft hover:text-ink",
              )}
            >
              {t(`scopes.level.${option}`)}
            </button>
          ))}
        </div>
      </div>
      <p
        className={cn(
          "mt-1 text-xs leading-relaxed",
          level === "none" ? "text-ink-faint" : "font-bold text-ink-soft",
        )}
      >
        {localized(described?.description)}
      </p>
    </div>
  );
}

/** What granted scopes mean, in the catalogue's words: sensitive ones set apart. */
export function ScopeMeanings({ scopes }: { scopes: dev.ScopeInfo[] }) {
  const { t } = useTranslation("dev");
  return (
    <ul className="flex flex-col gap-2">
      {scopes.map((scope) => (
        <li
          key={scope.scope}
          className={cn(
            "rounded-2xl px-3.5 py-2.5",
            scope.sensitive ? "border-2 border-ink bg-sun/25" : "bg-surface-2",
          )}
        >
          <p className="text-[15px] font-bold leading-relaxed">{localized(scope.description)}</p>
          <p className="mt-1 flex flex-wrap items-center gap-1.5">
            <Chip className="bg-surface font-mono">{scope.scope}</Chip>
            {scope.write && <Chip className="bg-surface">{t("scopes.includesRead")}</Chip>}
            {scope.sensitive && (
              <Chip className="bg-sun text-[#17122b]">{t("scopes.sensitive")}</Chip>
            )}
          </p>
        </li>
      ))}
    </ul>
  );
}
