import { Page, Skeleton } from "#/components/ui/page";
import { SwitchRow } from "#/components/ui/switch";
import { useTranslation } from "#/lib/use-translation";
import { useChangeReactionSettings, useReactionSettings } from "./api";

/**
 * What people may react with on the caller's own content: custom emoji at
 * all, and custom emoji marked sensitive. Each switch saves as it moves.
 */
export function ReactionSettingsScreen() {
  const { t } = useTranslation("engagement");
  const { data: settings } = useReactionSettings();
  const change = useChangeReactionSettings();

  return (
    <Page title={t("reactionSettings.title")} back="/settings">
      <p className="px-1 text-sm leading-relaxed text-ink-soft">{t("reactionSettings.lead")}</p>
      {settings ? (
        <div className="sticker mt-4 rounded-card bg-surface p-2">
          <SwitchRow
            label={t("reactionSettings.custom")}
            hint={t("reactionSettings.customHint")}
            checked={settings.customEmoji}
            onChange={(customEmoji) =>
              void change({ customEmoji, sensitiveEmoji: settings.sensitiveEmoji })
            }
          />
          <SwitchRow
            label={t("reactionSettings.sensitive")}
            hint={t("reactionSettings.sensitiveHint")}
            checked={settings.customEmoji && settings.sensitiveEmoji}
            disabled={!settings.customEmoji}
            onChange={(sensitiveEmoji) =>
              void change({ customEmoji: settings.customEmoji, sensitiveEmoji })
            }
          />
        </div>
      ) : (
        <Skeleton className="mt-4 h-36" />
      )}
    </Page>
  );
}
