import { boxShareUrl } from "@tokuly/core/ask";
import { Share2 } from "lucide-react-native";
import { useTranslation } from "react-i18next";
import { Share } from "react-native";
import { Button } from "#/components/button";
import { useMe } from "#/features/session/store";
import { SERVER_URL } from "#/lib/config";
import { useTheme } from "#/lib/theme";

/** Shares the viewer's own box: its public page, which needs no account to read. */
export function ShareBoxButton({ block }: { block?: boolean }) {
  const { t } = useTranslation("ask");
  const { colors } = useTheme();
  const handle = useMe()?.user?.handle;
  if (!handle) return null;
  const url = boxShareUrl(handle, SERVER_URL);
  return (
    <Button
      testID="ask-share"
      block={block}
      icon={<Share2 size={18} color={colors["on-brand"]} />}
      onPress={() => void Share.share({ message: t("share.shareText", { url }) })}
    >
      {t("inbox.share")}
    </Button>
  );
}
