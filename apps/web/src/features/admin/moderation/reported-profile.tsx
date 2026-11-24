import { useMutation } from "@tanstack/react-query";
import { type admin, common, create, identity } from "@tokuly/api";
import { BookHeart } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Button } from "#/components/ui/button";
import { ProfileBook } from "#/features/profile";
import { errorMessage } from "#/lib/errors";
import { moderationClient } from "../api";
import { Section } from "../ui/console-page";

/**
 * A reported account's プロフ帳 in full — fields and cards whatever their
 * audience, hidden cards included — for an open case about the account.
 * Opening it is written to the audit log first, so it takes a press.
 */
export function ReportedProfile({ moderationCase }: { moderationCase: admin.ModerationCase }) {
  const { t } = useTranslation("admin");
  const look = useMutation({
    mutationFn: () => moderationClient().getReportedProfile({ caseId: moderationCase.id }),
  });
  const data = look.data;
  const user = data?.user;
  return (
    <Section title={t("moderation.profileBook.title")}>
      {data && user ? (
        <>
          <ProfileBook
            profile={create(identity.ProfileSchema, { user, extras: data.extras })}
            user={user}
            isSelf={false}
            isFriend={false}
          />
          {data.extras?.visibility && (
            <dl className="mt-4 grid grid-cols-2 gap-x-4 gap-y-1 text-sm sm:grid-cols-3">
              {Object.entries(data.extras.visibility)
                .filter(([field]) => !field.startsWith("$"))
                .map(([field, value]) => (
                  <div key={field} className="flex gap-2">
                    <dt className="text-ink-faint">{field}</dt>
                    <dd className="font-bold">{common.Visibility[value as common.Visibility]}</dd>
                  </div>
                ))}
            </dl>
          )}
        </>
      ) : (
        <div className="flex flex-col items-start gap-2">
          <p className="text-sm text-ink-soft">{t("moderation.profileBook.lead")}</p>
          <Button
            size="sm"
            variant="secondary"
            icon={<BookHeart className="size-4" />}
            loading={look.isPending}
            onClick={() => look.mutate()}
          >
            {t("moderation.profileBook.open")}
          </Button>
          {look.error && (
            <p role="alert" className="text-sm font-bold text-danger">
              {errorMessage(look.error, { namespace: "admin" })}
            </p>
          )}
        </div>
      )}
    </Section>
  );
}
