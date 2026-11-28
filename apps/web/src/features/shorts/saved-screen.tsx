import { EmptyState, Page } from "#/components/ui/page";
import { Tabs } from "#/components/ui/tabs";
import { useTranslation } from "#/lib/use-translation";
import { ShortList } from "./short-tile";

export type SavedTab = "saved" | "liked";

/** `/shorts/saved`: the shorts the viewer saved, and the ones they liked. Both lists are theirs alone. */
export function SavedScreen({
  tab,
  onTabChange,
}: {
  tab: SavedTab;
  onTabChange(tab: SavedTab): void;
}) {
  const { t } = useTranslation("shorts");
  return (
    <Page title={t("saved.title")} back="/shorts" wide>
      <Tabs
        className="mb-4"
        value={tab}
        onChange={onTabChange}
        items={[
          { value: "saved", label: t("saved.tabs.saved") },
          { value: "liked", label: t("saved.tabs.liked") },
        ]}
      />
      {tab === "saved" ? (
        <ShortList
          key="saved"
          context={{ kind: "saved" }}
          empty={<EmptyState emoji="🔖" title={t("saved.empty")} body={t("saved.emptyBody")} />}
        />
      ) : (
        <ShortList
          key="liked"
          context={{ kind: "liked" }}
          empty={
            <EmptyState emoji="💜" title={t("saved.emptyLiked")} body={t("saved.emptyLikedBody")} />
          }
        />
      )}
    </Page>
  );
}
