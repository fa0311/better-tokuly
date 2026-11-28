import { useTranslation } from "react-i18next";
import { Page } from "#/components/ui/page";
import { CREDIT_GROUPS, type Credit, LICENSES } from "./credits";

/** `/settings/licenses`: who made the data, fonts, icons and libraries, and under which licence. */
export function LicensesScreen() {
  const { t } = useTranslation("licenses");
  return (
    <Page title={t("title")} back="/settings">
      <p className="px-1 text-sm leading-relaxed text-ink-soft">{t("lead")}</p>
      {CREDIT_GROUPS.map((group) => (
        <section key={group.id} className="mt-7">
          <h2 className="px-1 text-lg font-black">{t(`groups.${group.id}`)}</h2>
          <ul className="sticker mt-3 divide-y-2 divide-line rounded-card bg-surface">
            {group.credits.map((credit) => (
              <CreditRow key={credit.name} credit={credit} />
            ))}
          </ul>
        </section>
      ))}
      <p className="mt-7 px-1 text-sm text-ink-soft">{t("own")}</p>
    </Page>
  );
}

function CreditRow({ credit }: { credit: Credit }) {
  const { t } = useTranslation("licenses");
  return (
    <li className="flex flex-wrap items-baseline gap-x-3 gap-y-0.5 px-4 py-3">
      <div className="min-w-0 flex-1">
        <a href={credit.url} target="_blank" rel="noreferrer" className="font-bold underline">
          {credit.name}
        </a>
        {credit.use && <p className="text-sm text-ink-soft">{t(`use.${credit.use}`)}</p>}
        {credit.holder && <p className="text-xs text-ink-faint">{credit.holder}</p>}
      </div>
      <span className="text-sm font-bold">
        {credit.licenses.map((license, index) => (
          <span key={license}>
            {index > 0 && " / "}
            <a href={LICENSES[license]} target="_blank" rel="noreferrer" className="underline">
              {license}
            </a>
          </span>
        ))}
      </span>
    </li>
  );
}
