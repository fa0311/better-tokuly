import { toBinary } from "@bufbuild/protobuf";
import type { APIRequestContext, Page } from "@playwright/test";
import { create, fromDate, wallet } from "@tokuly/api";
import { type Account, adminAccount, createAccount, expect, refreshAccount, rpc, signInAs, test } from "./support";

const WALLET = "tokuly.wallet.v1.WalletService";
const EARNINGS = "tokuly.wallet.v1.EarningsService";
const WALLET_ADMIN = "tokuly.wallet.v1.WalletAdminService";

/** An amount as the `Coins` glyph writes it: the coin's "T", then the number. */
const coins = (amount: number, signed = false) => new RegExp(`^T${signed && amount > 0 ? "\\+" : ""}${amount.toLocaleString("en-US")}$`);

/** The balance in the hero of the wallet home. */
const hero = (page: Page) => page.getByRole("region", { name: "コイン残高" });

interface Balance {
  coins?: string;
  earnings?: string;
}

async function balanceOf(request: APIRequestContext, account: Account): Promise<Balance> {
  return (await rpc<{ balance: Balance }>(request, `${WALLET}/GetBalance`, {}, account)).balance;
}

/** Puts earned coins into an account, the way support would: an admin adjustment of the earnings balance. */
async function grantEarnings(request: APIRequestContext, to: Account, amount: number): Promise<void> {
  const admin = await adminAccount(request);
  await rpc(request, `${WALLET_ADMIN}/AdjustBalance`, { userId: to.userId, account: "LEDGER_ACCOUNT_EARNINGS", delta: String(amount), reason: "e2e: earnings for a payout test" }, admin);
}

interface Payout {
  id: string;
  status: string;
  reviewNote?: string;
  paymentReference?: string;
}

async function requestPayout(request: APIRequestContext, as: Account, amount: number): Promise<Payout> {
  const response = await rpc<{ payout: Payout }>(request, `${EARNINGS}/RequestPayout`, { coins: String(amount), method: "PAYOUT_METHOD_PAYPAL", destination: `${as.handle}@paypal.example`, note: "e2e" }, as);
  return response.payout;
}

async function payoutsOf(request: APIRequestContext, account: Account): Promise<Payout[]> {
  return (await rpc<{ payouts?: Payout[] }>(request, `${EARNINGS}/ListPayoutRequests`, {}, account)).payouts ?? [];
}

/** Takes a request a failed test may have left waiting out of the shared review queue. */
async function withdraw(request: APIRequestContext, as: Account, payoutId: string): Promise<void> {
  await rpc(request, `${EARNINGS}/CancelPayoutRequest`, { payoutId }, as).catch(() => {});
}

test.describe("wallet home", () => {
  test("claiming the daily bonus pays out once and shows in the balance and the history", async ({ page, request, me }) => {
    await page.goto("/wallet");
    await expect(hero(page).getByText(coins(0))).toBeVisible();

    const bonus = page.getByRole("region", { name: "ログインボーナス" });
    await bonus.getByRole("button", { name: /受け取る/ }).click();

    // The reward is announced, the claim is gone until tomorrow, and the streak has begun.
    await expect(bonus.getByText("ゲット！")).toBeVisible();
    await expect(bonus.getByText(coins(10, true)).first()).toBeVisible();
    await expect(bonus.getByRole("button", { name: /受け取る/ })).toHaveCount(0);
    await expect(bonus.getByText("1日連続")).toBeVisible();
    await expect(bonus.getByText(/次は.+後/)).toBeVisible();
    await expect(hero(page).getByText(coins(10))).toBeVisible();
    expect((await balanceOf(request, me)).coins).toBe("10");

    // The movement is in the recent list and in the full history.
    await expect(page.getByRole("button", { name: /ログインボーナス/ })).toBeVisible();
    await hero(page).getByRole("link", { name: "履歴" }).click();
    await expect(page.getByRole("heading", { name: "コインの履歴" })).toBeVisible();
    const row = page.getByRole("button", { name: /ログインボーナス/ });
    await expect(row.getByText(coins(10, true))).toBeVisible();
    await expect(page.getByRole("region", { name: "今日" })).toBeVisible();

    // Its details open on a tap, and the earnings filter leaves it out.
    await row.click();
    await expect(page.getByRole("dialog").getByText("連続日数")).toBeVisible();
    await page.getByRole("dialog").getByRole("button", { name: "close" }).click();
    await page.getByRole("tab", { name: "収益" }).click();
    await expect(page.getByText("収益はまだこれから！")).toBeVisible();

    // Coming back later the same day, the card remembers.
    await page.goto("/wallet");
    await expect(page.getByRole("region", { name: "ログインボーナス" }).getByText("今日のぶんは受け取ったよ")).toBeVisible();
  });

  test("coins that arrive while the wallet is open show up without a reload", async ({ page, request, me }) => {
    await page.goto("/wallet");
    await expect(hero(page).getByText(coins(0))).toBeVisible();
    await expect(page.getByText("まだ動きがないよ")).toBeVisible();

    // Bought on another device: the balance change is pushed to this one.
    const shop = await rpc<{ packages: { id: string; sku: string }[] }>(request, `${WALLET}/ListCoinPackages`, {}, me);
    const pack = shop.packages.find((candidate) => candidate.sku === "coins_500");
    expect(pack, "the seeded coins_500 package").toBeTruthy();
    await rpc(request, `${WALLET}/StartPurchase`, { packageId: pack?.id }, me);

    await expect(hero(page).getByText(coins(520))).toBeVisible({ timeout: 20_000 });
    // (A notification toast may announce the purchase too: look in the page itself.)
    await expect(page.getByRole("main").getByRole("button", { name: /コインを購入しました/ })).toBeVisible();
  });
});

test.describe("buying coins", () => {
  test("a package bought with the development provider lands in the balance and the purchase history", async ({ page, request, me }) => {
    await page.goto("/wallet/buy");
    await expect(page.getByRole("heading", { name: "コインを買う" })).toBeVisible();
    // Honest about what this environment is: nobody is charged.
    await expect(page.getByText("開発用の決済です。お金はかからず、コインがすぐに届きます。")).toBeVisible();
    await expect(page.getByText("おトク")).toBeVisible();

    await page.getByRole("button", { name: /^520コインのパック/ }).click();
    const dialog = page.getByRole("dialog", { name: "コインの購入" });
    await expect(dialog.getByText("請求なし")).toBeVisible();
    await expect(dialog.getByText(coins(20, true))).toBeVisible();
    await dialog.getByRole("button", { name: "テスト購入する" }).click();

    await expect(dialog.getByText("コインが届いたよ！")).toBeVisible();
    await expect(dialog.getByText(coins(520, true))).toBeVisible();
    await dialog.getByRole("button", { name: "完了" }).click();

    // The pill in the header follows the balance.
    await expect(page.getByRole("link", { name: /コイン残高 520/ })).toBeVisible();
    expect((await balanceOf(request, me)).coins).toBe("520");

    await page.getByRole("link", { name: "購入履歴を見る" }).click();
    await expect(page.getByRole("heading", { name: "購入履歴" })).toBeVisible();
    const purchase = page.getByRole("link", { name: /完了/ });
    await expect(purchase.getByText(coins(520))).toBeVisible();
    await expect(purchase.getByText(/開発用（テスト決済）/)).toBeVisible();

    await purchase.click();
    await expect(page.getByRole("heading", { name: "購入の詳細" })).toBeVisible();
    await expect(page.getByText("（請求なし）")).toBeVisible();
    await expect(page.getByText("購入ID")).toBeVisible();
  });

  test("an unknown purchase says so instead of failing", async ({ page, me: _me }) => {
    await page.goto("/wallet/purchases/00000000-0000-7000-8000-000000000000");
    await expect(page.getByText("この購入は見つかりませんでした")).toBeVisible();
  });

  // The development provider settles instantly, so a purchase that waits for a
  // payment cannot be produced by this server. These tests play the provider's
  // part by answering the purchase RPCs in the browser, to drive the flow a
  // hosted checkout takes: pay elsewhere, come back, confirm (or give up).
  test.describe("with a provider that has a hosted checkout (its answers are simulated)", () => {
    const PURCHASE_ID = "01900000-0000-7000-8000-00000000e2e0";
    const proto = (schema: Parameters<typeof toBinary>[0], message: Parameters<typeof toBinary>[1]) => ({
      status: 200,
      contentType: "application/proto",
      body: Buffer.from(toBinary(schema, message)),
    });
    const purchase = (status: wallet.PurchaseStatus) =>
      create(wallet.PurchaseSchema, {
        id: PURCHASE_ID,
        status,
        packageId: "pkg",
        sku: "coins_500",
        coins: 500n,
        bonusCoins: 20n,
        priceMinor: 500n,
        currency: "JPY",
        provider: "paynow",
        checkoutUrl: status === wallet.PurchaseStatus.PENDING ? "https://pay.example/checkout/e2e" : "",
        createdAt: fromDate(new Date()),
        expiresAt: fromDate(new Date(Date.now() + 45 * 60_000)),
      });

    async function startPendingPurchase(page: Page) {
      await page.route(`**/api/${WALLET}/ListCoinPackages`, (route) =>
        route.fulfill(
          proto(
            wallet.ListCoinPackagesResponseSchema,
            create(wallet.ListCoinPackagesResponseSchema, {
              packages: [{ id: "pkg", sku: "coins_500", coins: 500n, bonusCoins: 20n, priceMinor: 500n, currency: "JPY", active: true }],
              providers: ["paynow"],
            }),
          ),
        ),
      );
      await page.route(`**/api/${WALLET}/StartPurchase`, (route) =>
        route.fulfill(proto(wallet.StartPurchaseResponseSchema, create(wallet.StartPurchaseResponseSchema, { purchase: purchase(wallet.PurchaseStatus.PENDING), balance: { coins: 0n, revision: 1n } }))),
      );
      await page.goto("/wallet/buy");
      // A real provider: no development note, and the button names the price.
      await expect(page.getByText("開発用の決済です")).toHaveCount(0);
      await page.getByRole("button", { name: /^520コインのパック/ }).click();
      const dialog = page.getByRole("dialog", { name: "コインの購入" });
      await dialog.getByRole("button", { name: /500で購入する/ }).click();
      await expect(dialog.getByText("お支払いを完了してね")).toBeVisible();
      await expect(dialog.getByRole("link", { name: "決済ページへ" })).toHaveAttribute("href", "https://pay.example/checkout/e2e");
      await expect(dialog.getByText(/あと\d+分以内にお支払いしてね/)).toBeVisible();
      return dialog;
    }

    test("the buyer pays at the provider, comes back and confirms", async ({ page, me: _me }) => {
      let paid = false;
      await page.route(`**/api/${WALLET}/ConfirmPurchase`, (route) =>
        route.fulfill(
          proto(
            wallet.ConfirmPurchaseResponseSchema,
            create(wallet.ConfirmPurchaseResponseSchema, {
              purchase: purchase(paid ? wallet.PurchaseStatus.COMPLETED : wallet.PurchaseStatus.PENDING),
              balance: paid ? { coins: 520n, revision: 2n } : { coins: 0n, revision: 1n },
            }),
          ),
        ),
      );
      const dialog = await startPendingPurchase(page);

      // Asking before the money arrived changes nothing and says so.
      await dialog.getByRole("button", { name: "支払ったので確認する" }).click();
      await expect(page.getByText("まだお支払いが確認できていません")).toBeVisible();
      await expect(dialog.getByText("お支払いを完了してね")).toBeVisible();

      paid = true;
      await dialog.getByRole("button", { name: "支払ったので確認する" }).click();
      await expect(dialog.getByText("コインが届いたよ！")).toBeVisible();
      await dialog.getByRole("button", { name: "完了" }).click();
      await expect(page.getByRole("link", { name: /コイン残高 520/ })).toBeVisible();
    });

    test("the buyer gives up before paying", async ({ page, me: _me }) => {
      await page.route(`**/api/${WALLET}/CancelPurchase`, (route) =>
        route.fulfill(proto(wallet.CancelPurchaseResponseSchema, create(wallet.CancelPurchaseResponseSchema, { purchase: purchase(wallet.PurchaseStatus.CANCELLED) }))),
      );
      const dialog = await startPendingPurchase(page);

      await dialog.getByRole("button", { name: "購入をやめる" }).click();
      await expect(dialog.getByText("この購入はキャンセルされました")).toBeVisible();
      await expect(dialog.getByText("コインは増えていません。")).toBeVisible();
      // Trying again starts over at the confirmation.
      await dialog.getByRole("button", { name: "もう一度ためす" }).click();
      await expect(page.getByRole("dialog", { name: "コインの購入" }).getByRole("button", { name: /500で購入する/ })).toBeVisible();
    });
  });
});

test.describe("earnings", () => {
  test("a creator requests a payout within the limits, then takes it back", async ({ page, request, me }) => {
    await grantEarnings(request, me, 2500);
    await page.goto("/wallet/earnings");
    const available = page.getByRole("region", { name: "出金できる収益" });
    await expect(available.getByText(coins(2500))).toBeVisible();
    await expect(page.getByText("出金申請はまだないよ")).toBeVisible();

    await available.getByRole("button", { name: "出金を申請する" }).click();
    const form = page.getByRole("dialog", { name: "出金を申請する" });
    const amount = form.getByLabel("出金するコイン");
    await expect(amount).toHaveValue("2500");

    // Below the minimum, above what is there, then a valid amount.
    await amount.fill("500");
    await expect(form.getByText("出金は1,000コインからだよ")).toBeVisible();
    await amount.fill("9999");
    await expect(form.getByText("出金できるのは2,500コインまでだよ")).toBeVisible();
    await amount.fill("1200");
    await expect(form.getByText(/[¥￥]1,200/)).toBeVisible();

    // Where to send the money is required.
    await form.getByRole("button", { name: "申請する" }).click();
    await expect(form.getByText("受け取り先を入力してね")).toBeVisible();
    await form.getByLabel("受け取り先").fill("みずほ銀行 渋谷支店 普通 1234567 トクリー タロウ");
    await form.getByRole("button", { name: "申請する" }).click();
    await expect(page.getByText("出金を申請しました！")).toBeVisible();

    try {
      // The coins are locked, the request waits for review, and a second one has to wait.
      await expect(available.getByText(coins(1300))).toBeVisible();
      await expect(page.getByText("審査待ち")).toBeVisible();
      await expect(available.getByRole("button", { name: "出金を申請する" })).toBeDisabled();
      await expect(available.getByText(/進行中の申請があるよ/)).toBeVisible();
      expect((await balanceOf(request, me)).earnings).toBe("1300");

      await page.getByRole("button", { name: "申請を取り消す" }).click();
      const confirm = page.getByRole("dialog", { name: "出金申請を取り消しますか？" });
      await expect(confirm.getByText(/1,200コインは収益にもどります/)).toBeVisible();
      await confirm.getByRole("button", { name: "取り消す" }).click();
      await expect(page.getByText("申請を取り消しました")).toBeVisible();

      await expect(available.getByText(coins(2500))).toBeVisible();
      await expect(page.getByText("取り消し", { exact: true })).toBeVisible();
      await expect(available.getByRole("button", { name: "出金を申請する" })).toBeEnabled();
      expect((await balanceOf(request, me)).earnings).toBe("2500");
      expect((await payoutsOf(request, me))[0]?.status).toBe("PAYOUT_STATUS_CANCELLED");
    } finally {
      for (const payout of await payoutsOf(request, me)) await withdraw(request, me, payout.id);
    }
  });

  test("earnings below the minimum explain what is missing", async ({ page, request, me }) => {
    await grantEarnings(request, me, 400);
    await page.goto("/wallet/earnings");
    const available = page.getByRole("region", { name: "出金できる収益" });
    await expect(available.getByText(coins(400))).toBeVisible();
    await expect(available.getByRole("button", { name: "出金を申請する" })).toBeDisabled();
    await expect(available.getByText("あと600コインで出金できるよ！")).toBeVisible();
    // The period view is there even before the first transfer.
    await expect(page.getByRole("heading", { name: "収益の推移" })).toBeVisible();
    await page.getByRole("tab", { name: "12か月" }).click();
    await expect(page.getByText("この期間の収益はまだないよ", { exact: true })).toBeVisible();
  });

  test("a decision by staff reaches the open payout page with its reason", async ({ page, request, me }) => {
    await grantEarnings(request, me, 1500);
    const payout = await requestPayout(request, me, 1500);
    try {
      await page.goto(`/wallet/payouts/${payout.id}`);
      await expect(page.getByRole("heading", { name: "出金申請" })).toBeVisible();
      await expect(page.getByText("運営が内容を確認しています。もう少し待っててね！")).toBeVisible();

      const admin = await adminAccount(request);
      await rpc(request, `${WALLET_ADMIN}/RejectPayout`, { payoutId: payout.id, reviewNote: "口座名義がプロフィールと一致しません" }, admin);

      // Pushed live: no reload.
      const main = page.getByRole("main");
      await expect(main.getByText("却下されました", { exact: true })).toBeVisible({ timeout: 20_000 });
      await expect(main.getByText("口座名義がプロフィールと一致しません")).toBeVisible();
      await expect(main.getByText("1,500コインは収益にもどりました。")).toBeVisible();
      await expect(page.getByRole("button", { name: "申請を取り消す" })).toHaveCount(0);
    } finally {
      await withdraw(request, me, payout.id);
    }
  });
});

test.describe("staff", () => {
  test("a payout request is approved and then marked as paid", async ({ page, request }) => {
    const creator = await createAccount(request, "creator");
    await grantEarnings(request, creator, 3000);
    const payout = await requestPayout(request, creator, 2000);
    try {
      await signInAs(page, await adminAccount(request));
      // Straight to the request, as a link from a notification or a colleague would.
      await page.goto(`/admin/wallet?tab=payouts&payout=${payout.id}`);

      // Phones show the request in a sheet, wide screens beside the queue: either way it is the one place with these actions.
      const approve = page.getByRole("button", { name: "承認する" });
      await expect(page.getByText(`@${creator.handle}`).last()).toBeVisible();
      await expect(page.getByText(`${creator.handle}@paypal.example`)).toBeVisible();
      await approve.click();
      const approval = page.getByRole("dialog", { name: "この出金申請を承認しますか？" });
      await approval.getByLabel("申請者へのメッセージ（任意）").fill("おつかれさまです！");
      await approval.getByRole("button", { name: "承認する" }).click();
      await expect(page.getByText("承認しました")).toBeVisible();
      await expect(page.getByText("承認済み").first()).toBeVisible();

      await page.getByRole("button", { name: "支払い済みにする" }).click();
      const paid = page.getByRole("dialog", { name: "支払い済みにしますか？" });
      await paid.getByLabel("支払いの控え（任意）").fill("TX-E2E-0001");
      await paid.getByRole("button", { name: "支払い済みにする" }).click();
      await expect(page.getByText("支払い済みにしました")).toBeVisible();
      await expect(page.getByText("TX-E2E-0001")).toBeVisible();
      await expect(page.getByRole("button", { name: "却下する" })).toHaveCount(0);

      const [settled] = await payoutsOf(request, creator);
      expect(settled).toMatchObject({ status: "PAYOUT_STATUS_PAID", reviewNote: "おつかれさまです！", paymentReference: "TX-E2E-0001" });
      expect((await balanceOf(request, creator)).earnings).toBe("1000");
    } finally {
      await withdraw(request, creator, payout.id);
    }
  });

  test("a payout request is picked from the queue and rejected with a reason", async ({ page, request }) => {
    const creator = await createAccount(request, "creator");
    await grantEarnings(request, creator, 1200);
    const payout = await requestPayout(request, creator, 1200);
    try {
      await signInAs(page, await adminAccount(request));
      await page.goto("/admin/wallet");
      // The tab carries the number waiting for review.
      await expect(page.getByRole("tab", { name: /出金審査/ })).toContainText(/\d/);
      await page.getByRole("button", { name: new RegExp(`@${creator.handle}`) }).click();

      await page.getByRole("button", { name: "却下する" }).click();
      const dialog = page.getByRole("dialog", { name: "この出金申請を却下しますか？" });
      // No reason, no rejection.
      await dialog.getByRole("button", { name: "却下する" }).click();
      await expect(dialog.getByText("却下の理由を入力してください")).toBeVisible();
      await dialog.getByLabel("却下の理由").fill("本人確認が完了していません");
      await dialog.getByRole("button", { name: "却下する" }).click();
      await expect(page.getByText("却下しました")).toBeVisible();
      await expect(page.getByRole("definition").filter({ hasText: "本人確認が完了していません" })).toBeVisible();

      const [rejected] = await payoutsOf(request, creator);
      expect(rejected).toMatchObject({ status: "PAYOUT_STATUS_REJECTED", reviewNote: "本人確認が完了していません" });
      // The coins are back with the creator.
      expect((await balanceOf(request, creator)).earnings).toBe("1200");
    } finally {
      await withdraw(request, creator, payout.id);
    }
  });

  test("a user's wallet is found by search and corrected with a reason, which lands in the log", async ({ page, request }) => {
    const user = await createAccount(request, "holder");
    await rpc(request, `${WALLET}/ClaimDailyBonus`, {}, user);
    await signInAs(page, await adminAccount(request));
    await page.goto("/admin/wallet");
    await page.getByRole("tab", { name: "ユーザー" }).click();

    await page.getByRole("textbox", { name: "ユーザーを検索" }).fill(user.handle);
    await page.getByRole("button", { name: new RegExp(`@${user.handle}`) }).click();
    await expect(page.getByText("購入したコイン（累計）")).toBeVisible();
    await expect(page.getByRole("button", { name: /ログインボーナス/ })).toBeVisible();

    await page.getByRole("button", { name: "残高を調整" }).click();
    const dialog = page.getByRole("dialog", { name: "残高の調整" });
    await dialog.getByLabel("コインの数").fill("500");
    // A correction without a reason does not get to the confirmation.
    await dialog.getByRole("button", { name: "内容を確認する" }).click();
    await expect(dialog.getByText("理由を入力してください")).toBeVisible();
    const reason = `e2e 補填 ${user.handle}`;
    await dialog.getByLabel("理由").fill(reason);
    await dialog.getByRole("button", { name: "内容を確認する" }).click();

    const review = page.getByRole("dialog", { name: "この内容で調整しますか？" });
    await expect(review.getByText(coins(500, true))).toBeVisible();
    await expect(review.getByText("10 → 510")).toBeVisible();
    await review.getByRole("button", { name: "調整を実行する" }).click();
    await expect(page.getByText("残高を調整しました")).toBeVisible();

    // The wallet, its ledger and the user's own balance all show the correction.
    await expect(page.getByText(coins(510))).toBeVisible();
    await expect(page.getByRole("button", { name: /運営による残高の調整/ })).toBeVisible();
    expect((await balanceOf(request, user)).coins).toBe("510");

    await page.getByRole("tab", { name: "調整ログ" }).click();
    const entry = page.getByRole("listitem").filter({ hasText: reason });
    await expect(entry.getByText(`@${user.handle}`)).toBeVisible();
    await expect(entry.getByText(coins(500, true))).toBeVisible();
  });

  test("a coin package is created, sold in the shop, edited and retired", async ({ page, request }) => {
    const admin = await adminAccount(request);
    const sku = `e2e_${Date.now().toString(36)}${Math.floor(Math.random() * 1e4)}`;
    try {
      await signInAs(page, admin);
      await page.goto("/admin/wallet?tab=packages");
      await page.getByRole("button", { name: "新しいパック" }).click();
      const editor = page.getByRole("dialog", { name: "新しいコインパック" });
      // Nothing is saved while the form is incomplete.
      await editor.getByRole("button", { name: "保存する" }).click();
      await expect(editor.getByText("半角英小文字・数字・_ - . で3〜64文字にしてください")).toBeVisible();
      await editor.getByLabel("SKU").fill(sku);
      await editor.getByLabel("コイン", { exact: true }).fill("77");
      await editor.getByLabel("ボーナスコイン").fill("3");
      await editor.getByLabel("価格（通貨の最小単位）").fill("90");
      await expect(editor.getByText(/表示: [¥￥]90/)).toBeVisible();
      await editor.getByRole("button", { name: "保存する" }).click();
      await expect(page.getByText("パックを作成しました")).toBeVisible();

      const row = page.getByRole("listitem").filter({ hasText: sku });
      await expect(row.getByText("販売中")).toBeVisible();

      // It is on the shelf for buyers.
      await page.goto("/wallet/buy");
      await expect(page.getByRole("button", { name: /^80コインのパック/ })).toBeVisible();

      await page.goto("/admin/wallet?tab=packages");
      await row.getByRole("button", { name: `${sku} を編集` }).click();
      const edit = page.getByRole("dialog", { name: "コインパックを編集" });
      await expect(edit.getByLabel("SKU")).toHaveValue(sku);
      await edit.getByLabel("ボーナスコイン").fill("13");
      await edit.getByRole("button", { name: "保存する" }).click();
      await expect(page.getByText("パックを保存しました")).toBeVisible();
      await expect(row.getByText("+13")).toBeVisible();

      await row.getByRole("button", { name: `${sku} を販売終了にする` }).click();
      await page.getByRole("dialog", { name: `${sku} を販売終了にしますか？` }).getByRole("button", { name: "販売終了" }).click();
      await expect(page.getByText("販売終了にしました")).toBeVisible();
      await expect(row.getByRole("button", { name: `${sku} の販売を再開する` })).toBeVisible();

      await page.goto("/wallet/buy");
      await expect(page.getByRole("button", { name: /^520コインのパック/ })).toBeVisible();
      await expect(page.getByRole("button", { name: /^90コインのパック/ })).toHaveCount(0);
    } finally {
      // Never leave a test package on sale in the shared shop.
      const all = await rpc<{ packages: { id: string; sku: string; active?: boolean }[] }>(request, `${WALLET_ADMIN}/ListAllCoinPackages`, {}, admin);
      const leftover = all.packages.find((pkg) => pkg.sku === sku && pkg.active);
      if (leftover) await rpc(request, `${WALLET_ADMIN}/SaveCoinPackage`, { package: { ...leftover, active: false } }, admin);
    }
  });

  test("a moderator reviews payouts and looks up wallets, but cannot adjust balances or edit the shop", async ({ page, request }) => {
    const admin = await adminAccount(request);
    const moderator = await createAccount(request, "moderator");
    await rpc(request, "tokuly.admin.v1.UserAdminService/SetUserRoles", { userId: moderator.userId, roles: ["moderator"], note: "e2e" }, admin);
    await signInAs(page, await refreshAccount(request, moderator));

    await page.goto("/admin/wallet");
    await expect(page.getByRole("tab", { name: /出金審査/ })).toBeVisible();
    await expect(page.getByRole("tab", { name: "ユーザー" })).toBeVisible();
    await expect(page.getByRole("tab", { name: "調整ログ" })).toHaveCount(0);
    await expect(page.getByRole("tab", { name: "コインパック" })).toHaveCount(0);

    await page.goto(`/admin/wallet?tab=users&user=${admin.userId}`);
    await expect(page.getByText("購入したコイン（累計）")).toBeVisible();
    await expect(page.getByRole("button", { name: "残高を調整" })).toHaveCount(0);
  });
});
