import type { Page } from "@playwright/test";

/**
 * Holds back the answers to `procedure` that the page asks for from now on.
 * Each request reaches the server at once, so its answer is made then; the
 * page receives it only on `release()`, after whatever is pushed meanwhile:
 * a slow read, answered before the push it then arrives after.
 */
export async function holdAnswers(page: Page, procedure: string) {
  const held: (() => Promise<void>)[] = [];
  let holding = true;
  await page.route(`**/api/${procedure}`, async (route) => {
    const response = await route.fetch();
    // A read the page gave up on (replaced, or nobody watches it) has nobody to answer.
    const answer = () => route.fulfill({ response }).catch(() => {});
    if (holding) held.push(answer);
    else await answer();
  });
  return {
    /** How many answers the server made and the page has not received. */
    held: () => held.length,
    /** Hands the page what was held and stops holding; resolves once the page has had a moment with it. */
    async release() {
      holding = false;
      await Promise.all(held.splice(0).map((answer) => answer()));
      // The answer is in the page; give it a moment to land in the cache and on screen.
      await page.waitForTimeout(500);
    },
  };
}
