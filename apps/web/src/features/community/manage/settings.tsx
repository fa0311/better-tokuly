import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { community } from "@tokuly/api";
import { can, communityKeys, refreshMembership, storeCommunity } from "@tokuly/core/community";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { TextField } from "#/components/ui/field";
import { errorMessage } from "#/lib/errors";
import { communityClient, NS } from "../api";
import {
  CommunityFields,
  draftFields,
  draftOf,
  draftQuestions,
  draftRules,
  draftTags,
  useHandleCheck,
} from "../community-form";
import { Notice, SectionTitle } from "../parts";

/** The community's profile, rules, join questions, visibility and switches; closing it for good. */
export function SettingsSection({ community: c }: { community: community.Community }) {
  const { t } = useTranslation(NS);
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const [draft, setDraft] = useState(() => draftOf(c));
  const [closing, setClosing] = useState(false);
  const [confirmHandle, setConfirmHandle] = useState("");
  const check = useHandleCheck(draft.handle, c.handle);

  const save = useMutation({
    mutationFn: () =>
      communityClient().updateCommunity({
        communityId: c.id,
        ...draftFields(draft),
        tags: { values: draftTags(draft) },
        rules: { rules: draftRules(draft) },
        joinQuestions: { values: draftQuestions(draft) },
        iconAssetId: draft.icon === undefined ? undefined : (draft.icon?.assetId ?? ""),
        bannerAssetId: draft.banner === undefined ? undefined : (draft.banner?.assetId ?? ""),
      }),
    onSuccess: ({ community: next }) => {
      if (!next) return;
      storeCommunity(queryClient, next);
      void queryClient.invalidateQueries({ queryKey: communityKeys.my });
      setDraft(draftOf(next));
      toast.success(t("saved"));
      // A new handle is a new address.
      if (next.handle !== c.handle)
        void navigate({
          to: "/g/$handle/manage/$section",
          params: { handle: next.handle, section: "settings" },
          replace: true,
        });
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: NS })),
  });
  const close = useMutation({
    mutationFn: () => communityClient().deleteCommunity({ communityId: c.id }),
    onSuccess: () => {
      queryClient.removeQueries({ queryKey: communityKeys.community(c.handle) });
      refreshMembership(queryClient, c.id);
      toast(t("manage.settings.closed"));
      void navigate({ to: "/communities" });
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: NS })),
  });
  const ready =
    Boolean(draft.name.trim()) && draft.handle.length >= 3 && check.settled && check.available;

  return (
    <div>
      {draft.visibility !== c.visibility && (
        <Notice tone="warn" className="mb-4">
          {t("manage.settings.visibilityChange")}
        </Notice>
      )}
      <CommunityFields draft={draft} onChange={setDraft} current={c} />
      <Button
        block
        size="lg"
        className="mt-6"
        loading={save.isPending}
        disabled={!ready}
        onClick={() => save.mutate()}
      >
        {t("save")}
      </Button>

      {can(c, community.Permission.DELETE_COMMUNITY) && (
        <>
          <SectionTitle>{t("manage.settings.danger")}</SectionTitle>
          <div className="rounded-card border-2 border-danger p-4">
            <p className="font-bold">{t("manage.settings.close")}</p>
            <p className="mt-1 text-sm text-ink-soft">{t("manage.settings.closeBody")}</p>
            <Button variant="danger" size="sm" className="mt-3" onClick={() => setClosing(true)}>
              {t("manage.settings.close")}
            </Button>
          </div>
          <ConfirmDialog
            open={closing}
            onOpenChange={(open) => {
              setClosing(open);
              setConfirmHandle("");
            }}
            title={t("manage.settings.closeTitle", { name: c.name })}
            description={t("manage.settings.closeConfirm", { handle: c.handle })}
            confirmLabel={t("manage.settings.close")}
            danger
            loading={close.isPending}
            onConfirm={() => confirmHandle.trim() === c.handle && close.mutate()}
          >
            <TextField
              aria-label={t("form.handle")}
              value={confirmHandle}
              placeholder={c.handle}
              autoCapitalize="off"
              autoComplete="off"
              onChange={(event) => setConfirmHandle(event.target.value)}
            />
          </ConfirmDialog>
        </>
      )}
    </div>
  );
}
