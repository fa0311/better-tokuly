import type { notify } from "@tokuly/api";
import { BellRing, Inbox, Lock } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Card } from "#/components/ui/card";
import { CategoryIcon } from "../category";
import { usePreferenceActions } from "../preferences";
import { Toggle } from "../toggle";

/** Per category: keep it in the inbox, and/or push it. */
export function CategoriesCard({ preferences }: { preferences: notify.Preferences }) {
  const { t } = useTranslation("notifications");
  const actions = usePreferenceActions();

  return (
    <Card className="px-3 sm:px-5">
      <div className="px-2 sm:px-0">
        <h2 className="text-lg font-black leading-snug">{t("categories.title")}</h2>
        <p className="mt-1 text-sm leading-relaxed text-ink-soft">{t("categories.lead")}</p>
      </div>

      {/* Column headings for the two switches of every row. */}
      <div aria-hidden className="mt-4 flex items-end justify-end gap-2 px-2 pb-1 text-[11px] font-black text-ink-soft sm:px-0">
        <span className="flex w-14 flex-col items-center gap-0.5">
          <Inbox className="size-4" />
          {t("categories.inbox")}
        </span>
        <span className="flex w-14 flex-col items-center gap-0.5">
          <BellRing className="size-4" />
          {t("categories.push")}
        </span>
      </div>

      <ul className="flex flex-col">
        {preferences.categories.map((category) => (
          // The description runs under the switches too, so narrow screens keep it to a line or two.
          <li key={category.category} className="grid grid-cols-[auto_minmax(0,1fr)_auto] items-center gap-x-3 rounded-2xl px-2 py-2.5 hover:bg-surface-2 sm:-mx-2" data-category={category.category}>
            <CategoryIcon category={category.category} size="sm" />
            <p className="font-bold leading-snug">{category.label}</p>
            <div className="flex shrink-0 gap-2">
              <Toggle
                label={t("categories.inboxOf", { category: category.label })}
                checked={category.inbox}
                disabled={category.inboxLocked}
                onChange={(inbox) => actions.setCategory.mutate({ category: category.category, inbox })}
              />
              <Toggle label={t("categories.pushOf", { category: category.label })} checked={category.push} onChange={(push) => actions.setCategory.mutate({ category: category.category, push })} />
            </div>
            <div className="col-span-2 col-start-2 mt-1 text-xs leading-snug text-ink-soft">
              <p>{category.description}</p>
              {category.inboxLocked && (
                <p className="mt-0.5 flex items-center gap-1 font-bold text-ink-faint">
                  <Lock className="size-3.5 shrink-0" aria-hidden />
                  {t("categories.locked")}
                </p>
              )}
            </div>
          </li>
        ))}
      </ul>
    </Card>
  );
}
