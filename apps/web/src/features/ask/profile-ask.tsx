import { Link } from "@tanstack/react-router";
import type { common } from "@tokuly/api";
import { useTranslation } from "react-i18next";
import { useAnswers, useBox } from "./api";
import { SectionTitle } from "./parts";
import { QaCard } from "./qa-card";

/** How many answers a profile shows before "more". */
const PREVIEW = 3;

/**
 * The profile's way into a question box: "質問する" on someone whose box is
 * open, "質問箱" on one's own profile. Renders nothing when there is no box
 * to go to.
 */
export function ProfileAskButton({
  user,
  isSelf,
}: {
  user: Pick<common.UserSummary, "id" | "handle">;
  isSelf: boolean;
}) {
  const { t } = useTranslation("ask");
  const { data: box } = useBox(isSelf ? undefined : user.id);
  const className =
    "pressable sticker inline-flex h-11 items-center gap-2 rounded-full bg-sun px-5 font-bold text-[#17122b]";
  if (isSelf) {
    return (
      <Link to="/ask" className={className}>
        💌 {t("profile.myBox")}
      </Link>
    );
  }
  if (!box?.open) return null;
  return (
    <Link to="/ask/u/$handle" params={{ handle: user.handle }} className={className}>
      💌 {t("profile.ask")}
    </Link>
  );
}

/** The newest published answers of a person, for their profile. Renders nothing while there are none. */
export function ProfileAnswers({ user }: { user: Pick<common.UserSummary, "id" | "handle"> }) {
  const { t } = useTranslation("ask");
  const answers = useAnswers(user.id);
  const questions = answers.data?.pages[0]?.questions ?? [];
  if (questions.length === 0) return null;
  const more = questions.length > PREVIEW || Boolean(answers.hasNextPage);
  return (
    <section data-testid="profile-answers">
      <SectionTitle
        action={
          more ? (
            <Link
              to="/ask/u/$handle"
              params={{ handle: user.handle }}
              className="text-sm font-bold text-brand"
            >
              {t("profile.more")}
            </Link>
          ) : undefined
        }
      >
        💌 {t("profile.title")}
      </SectionTitle>
      <div className="flex flex-col gap-4">
        {questions.slice(0, PREVIEW).map((question) => (
          <QaCard key={question.id} question={question} />
        ))}
      </div>
    </section>
  );
}
