import { readFile } from "node:fs/promises";
import type { APIRequestContext, Locator, Page } from "@playwright/test";
import { type Account, expect, rpc, test } from "./support";

const ACCOUNT = "tokuly.identity.v1.AccountService";

interface Appearance {
  accentColor?: string;
  customColors?: { light?: Record<string, string>; dark?: Record<string, string> };
  [field: string]: unknown;
}

const appearanceOf = async (request: APIRequestContext, as: Account) =>
  (await rpc<{ settings: { appearance: Appearance } }>(request, `${ACCOUNT}/GetSettings`, {}, as))
    .settings.appearance;

/** A token as the app (or the preview) uses it right now. */
const token = (target: Locator, name: string) =>
  target.evaluate(
    (element, name) => getComputedStyle(element).getPropertyValue(`--${name}`).trim(),
    name,
  );

/** Presses a control by keyboard only: focus it, then Enter. */
async function press(control: Locator) {
  await control.focus();
  await control.press("Enter");
}

/** Replaces what a field holds by typing. */
async function typeInto(page: Page, field: Locator, text: string) {
  await field.focus();
  await field.press("ControlOrMeta+a");
  await page.keyboard.type(text);
}

test("the theme editor recolours the app, checks contrast and saves to the account, by keyboard alone", async ({
  page,
  me,
}) => {
  await page.goto("/settings/appearance");
  await press(page.getByRole("link", { name: /テーマエディタ/ }));
  await expect(page.getByRole("heading", { level: 1, name: "テーマエディタ" })).toBeVisible();

  const html = page.locator("html");
  const preview = page.getByTestId("theme-preview");
  const report = page.getByTestId("contrast-report");
  await expect(report).toContainText("どの文字もちゃんと読めるよ");

  // Soft text too pale to read: the whole app and the preview follow, and the check warns.
  const softText = page.getByRole("textbox", { name: "うすい文字のカラーコード" });
  await typeInto(page, softText, "#d0d0d0");
  await expect.poll(() => token(html, "ink-soft")).toBe("#d0d0d0");
  await expect.poll(() => token(preview, "ink-soft")).toBe("#d0d0d0");
  await expect(report).toContainText("読みにくい組み合わせが4つあるよ");
  await expect(report).toContainText("「うすい文字」と「カード」");
  await expect
    .poll(async () => (await appearanceOf(page.request, me)).customColors?.light)
    .toEqual({ "ink-soft": "#d0d0d0" });

  // One token back to the theme's own colour.
  await press(page.getByRole("button", { name: "うすい文字を元に戻す" }));
  await expect.poll(() => token(html, "ink-soft")).toBe("#5d5778");
  await expect(report).toContainText("どの文字もちゃんと読めるよ");

  // The dark theme is edited in the preview while the app stays light.
  await press(page.getByRole("tab", { name: "ダーク" }));
  await typeInto(page, page.getByRole("textbox", { name: "背景のカラーコード" }), "000000");
  await expect.poll(() => token(preview, "bg")).toBe("#000000");
  expect(await token(html, "bg")).toBe("#f4f1ff");
  await expect
    .poll(async () => (await appearanceOf(page.request, me)).customColors)
    .toEqual({ dark: { bg: "#000000" } });

  // Export: a theme file with what changed.
  const download = page.waitForEvent("download");
  await press(page.getByRole("button", { name: "JSONで書き出す" }));
  const file = await download;
  expect(file.suggestedFilename()).toBe("tokuly-theme.json");
  const exported = JSON.parse(await readFile(await file.path(), "utf8"));
  expect(exported).toEqual({
    format: "tokuly-theme",
    version: 1,
    light: {},
    dark: { bg: "#000000" },
  });

  // Reset everything, confirmed in the dialog.
  await press(page.getByRole("button", { name: "ぜんぶ元に戻す" }));
  await press(page.getByRole("dialog").getByRole("button", { name: "元に戻す" }));
  await expect.poll(() => token(preview, "bg")).toBe("#0d0b14");
  await expect
    .poll(async () => (await appearanceOf(page.request, me)).customColors ?? {})
    .toEqual({});

  // Import: what is not a theme is refused; a theme replaces the colours.
  await press(page.getByRole("button", { name: "JSONを読み込む" }));
  const dialog = page.getByRole("dialog");
  const json = dialog.getByRole("textbox", { name: "テーマのJSON" });
  await json.focus();
  await page.keyboard.insertText('{"light":{}}');
  await press(dialog.getByRole("button", { name: "読み込む", exact: true }));
  await expect(dialog).toContainText("Tokulyのテーマファイルじゃないみたい");
  await typeInto(page, json, "");
  await page.keyboard.insertText(
    JSON.stringify({ ...exported, light: { ink: "#2a0a3a", sparkle: "#ffffff" } }),
  );
  await press(dialog.getByRole("button", { name: "読み込む", exact: true }));
  await expect(dialog).toBeHidden();
  await expect(page.getByText("この版のTokulyにない色が1こあったので、とばしたよ。")).toBeVisible();
  await expect.poll(() => token(html, "ink")).toBe("#2a0a3a");
  await expect
    .poll(async () => (await appearanceOf(page.request, me)).customColors)
    .toEqual({ light: { ink: "#2a0a3a" }, dark: { bg: "#000000" } });

  // Still there after a reload.
  await page.reload();
  await expect.poll(() => token(html, "ink")).toBe("#2a0a3a");
});

test("an account's theme follows it to a new browser, and a new accent takes the brand back", async ({
  page,
  me,
  request,
}) => {
  const appearance = await appearanceOf(request, me);
  await rpc(
    request,
    `${ACCOUNT}/UpdateSettings`,
    {
      appearance: {
        ...appearance,
        customColors: { light: { bg: "#fff4e0", brand: "#d02060" }, dark: { ink: "#ffe0f0" } },
      },
    },
    me,
  );

  await page.goto("/settings/appearance");
  const html = page.locator("html");
  await expect.poll(() => token(html, "bg")).toBe("#fff4e0");
  await expect.poll(() => token(html, "brand")).toBe("#d02060");

  // The accent sets the brand again; the other colours stay.
  await press(page.getByRole("radio", { name: "みどり" }));
  await expect.poll(() => token(html, "brand")).toBe("#12a36b");
  expect(await token(html, "bg")).toBe("#fff4e0");
  await expect
    .poll(async () => (await appearanceOf(page.request, me)).customColors)
    .toEqual({ light: { bg: "#fff4e0" }, dark: { ink: "#ffe0f0" } });

  // The editor opens on the theme the app shows, with the accent's brand as its base.
  await page.goto("/settings/appearance/theme");
  await expect(page.getByRole("tab", { name: "ライト" })).toHaveAttribute("aria-selected", "true");
  await expect(page.getByRole("textbox", { name: "ブランド色のカラーコード" })).toHaveValue(
    "#12a36b",
  );
  await expect(page.getByRole("textbox", { name: "背景のカラーコード" })).toHaveValue("#fff4e0");
});
