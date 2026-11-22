import type { stamp } from "@tokuly/api";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "#/components/ui/button";
import { Coins } from "#/components/ui/coins";
import { Dialog } from "#/components/ui/dialog";
import { TextArea, TextField } from "#/components/ui/field";
import { SwitchRow } from "#/components/ui/switch";
import { parseKeywords } from "./stamp-meta";

/** A pack's listing as edited in the form. */
export interface PackListing {
  name: string;
  description: string;
  tags: string[];
  /** Coins; 0 = free. */
  price: number;
}

/** The limits the form checks against; the server is the judge. */
export type PriceLimits = Pick<stamp.CreatorProfile, "minPrice" | "maxPrice" | "feePercent">;

/** What a creator keeps of one sale, after the platform's share. */
export function netEarnings(price: number, feePercent: number): number {
  return Math.floor((price * (100 - feePercent)) / 100);
}

/**
 * The create / edit form of a pack's listing: name, description, tags and
 * price. `onSubmit` saves; the dialog closes when it resolves.
 */
export function PackFormDialog({
  open,
  onOpenChange,
  title,
  submitLabel,
  initial,
  limits,
  official,
  onSubmit,
}: {
  open: boolean;
  onOpenChange(open: boolean): void;
  title: string;
  submitLabel: string;
  initial?: PackListing;
  limits?: PriceLimits;
  /** Official packs earn nobody anything, so the earnings line is left out. */
  official?: boolean;
  onSubmit(listing: PackListing): Promise<unknown>;
}) {
  const { t } = useTranslation("stamps");
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [tags, setTags] = useState("");
  const [paid, setPaid] = useState(false);
  const [price, setPrice] = useState("");
  const [saving, setSaving] = useState(false);

  // Every opening starts from the pack as it is now.
  const [wasOpen, setWasOpen] = useState(false);
  if (open !== wasOpen) {
    setWasOpen(open);
    if (open) {
      setName(initial?.name ?? "");
      setDescription(initial?.description ?? "");
      setTags(initial?.tags.map((tag) => `#${tag}`).join(" ") ?? "");
      setPaid((initial?.price ?? 0) > 0);
      setPrice(initial?.price ? String(initial.price) : "");
    }
  }

  const min = limits?.minPrice ?? 10;
  const max = limits?.maxPrice ?? 10_000;
  const amount = Number.parseInt(price, 10);
  const priceError =
    paid && (!Number.isFinite(amount) || amount < min || amount > max)
      ? t("form.priceRange", { min, max: max.toLocaleString("en-US") })
      : undefined;
  const valid = name.trim().length > 0 && !priceError;

  const submit = async () => {
    if (!valid) return;
    setSaving(true);
    try {
      await onSubmit({
        name: name.trim(),
        description: description.trim(),
        tags: parseKeywords(tags),
        price: paid ? amount : 0,
      });
      onOpenChange(false);
    } catch {
      // `onSubmit` reports its own failure; the form stays open with what was typed.
    } finally {
      setSaving(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={(next) => !saving && onOpenChange(next)} title={title}>
      <form
        className="flex flex-col gap-4"
        onSubmit={(event) => {
          event.preventDefault();
          void submit();
        }}
      >
        <TextField
          label={t("form.name")}
          placeholder={t("form.namePlaceholder")}
          value={name}
          maxLength={40}
          required
          autoFocus
          onChange={(event) => setName(event.target.value)}
        />
        <TextArea
          label={t("form.description")}
          placeholder={t("form.descriptionPlaceholder")}
          value={description}
          maxLength={500}
          rows={3}
          onChange={(event) => setDescription(event.target.value)}
        />
        <TextField
          label={t("form.tags")}
          placeholder={t("form.tagsPlaceholder")}
          hint={t("form.tagsHint")}
          value={tags}
          maxLength={280}
          onChange={(event) => setTags(event.target.value)}
        />
        <div className="rounded-2xl border-2 border-line p-1">
          <SwitchRow
            label={t("form.paid")}
            hint={t("form.paidHint")}
            checked={paid}
            onChange={setPaid}
          />
          {paid && (
            <div className="flex flex-col gap-2 px-3 pb-3">
              <TextField
                label={t("form.price")}
                type="number"
                inputMode="numeric"
                min={min}
                max={max}
                step={1}
                value={price}
                error={price ? priceError : undefined}
                hint={t("form.priceRange", { min, max: max.toLocaleString("en-US") })}
                onChange={(event) => setPrice(event.target.value)}
              />
              {!official && limits && !priceError && (
                <p className="flex flex-wrap items-center gap-1.5 px-1 text-sm font-bold text-ink-soft">
                  {t("form.earnings", { fee: limits.feePercent })}
                  <Coins
                    amount={netEarnings(amount, limits.feePercent)}
                    size="sm"
                    className="text-ink"
                  />
                </p>
              )}
            </div>
          )}
        </div>
        <div className="flex justify-end gap-2">
          <Button variant="ghost" onClick={() => onOpenChange(false)} disabled={saving}>
            {t("common:actions.cancel")}
          </Button>
          <Button type="submit" loading={saving} disabled={!valid}>
            {submitLabel}
          </Button>
        </div>
      </form>
    </Dialog>
  );
}
