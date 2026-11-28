import { EmptyState } from "#/components/ui/page";
import { useTranslation } from "#/lib/use-translation";

/** What the conversation pane shows on wide screens while no conversation is open. */
export function ChatHome() {
  const { t } = useTranslation("chat");
  return (
    <div className="grid h-full place-items-center">
      <EmptyState emoji="💌" title={t("home.title")} body={t("home.body")} />
    </div>
  );
}
