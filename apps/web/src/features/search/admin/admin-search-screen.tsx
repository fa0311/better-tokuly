import { useTranslation } from "react-i18next";
import { EmptyState } from "#/components/ui/page";
import { Tabs } from "#/components/ui/tabs";
import { useStaffAccess } from "#/features/admin";
import { STAFF_TABS, type StaffTab } from "./admin-search-search";
import { BlockedTerms } from "./blocked-terms";
import { Documents } from "./documents";
import { Indexes } from "./indexes";
import { StaffPage } from "./parts";
import { Synonyms } from "./synonyms";

/**
 * `/admin/search`: the search engine as its operators see it — the state of
 * every index and rebuilding one, taking a document out, and the word lists
 * (blocked terms, synonym groups). Everything here needs the `admin` role.
 */
export function AdminSearchScreen({ tab, onTab }: { tab: StaffTab; onTab(tab: StaffTab): void }) {
  const { t } = useTranslation("search");
  const access = useStaffAccess();
  return (
    <StaffPage title={t("staff.title")}>
      {!access.admin ? (
        <EmptyState emoji="🔐" title={t("staff.gate.title")} body={t("staff.gate.body")} />
      ) : (
        <>
          <Tabs<StaffTab>
            value={tab}
            onChange={onTab}
            items={STAFF_TABS.map((value) => ({ value, label: t(`staff.tabs.${value}`) }))}
          />
          <div className="mt-4">
            {tab === "indexes" && <Indexes />}
            {tab === "documents" && <Documents />}
            {tab === "blocked" && <BlockedTerms />}
            {tab === "synonyms" && <Synonyms />}
          </div>
        </>
      )}
    </StaffPage>
  );
}
