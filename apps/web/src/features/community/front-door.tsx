import { Lock } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Page } from "#/components/ui/page";
import { EmojiText } from "#/features/emoji";
import { Facts, Rules } from "./about";
import { NS } from "./api";
import { useCurrentCommunity } from "./context";
import { CommunityHeader } from "./header";
import { Notice } from "./parts";

/**
 * What someone sees of a community they may not read: its front page (what
 * it is, its rules, how to get in) and nothing of what is inside.
 */
export function FrontDoor() {
  const { t } = useTranslation(NS);
  const c = useCurrentCommunity();
  return (
    <Page title={<EmojiText text={c.name} />} back="/communities">
      <CommunityHeader community={c} />
      <Notice icon={<Lock />} className="mt-4">
        {t("frontDoor.membersOnly")}
      </Notice>
      {c.joinQuestions.length > 0 && (
        <section className="mt-4 rounded-card bg-surface p-4">
          <h3 className="font-black">{t("frontDoor.questions")}</h3>
          <ul className="mt-2 list-disc pl-5 text-sm leading-relaxed text-ink-soft">
            {c.joinQuestions.map((question) => (
              <li key={question}>{question}</li>
            ))}
          </ul>
        </section>
      )}
      <div className="mt-4 flex flex-col gap-4">
        <Rules community={c} />
        <Facts community={c} />
      </div>
    </Page>
  );
}
