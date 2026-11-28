import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import type { APIRequestContext } from "@playwright/test";
import { discPicture, type Rgb, uploadPicture } from "./pictures";
import { type Account, rpc } from "./support";

// Moments for the tests: a prompt that reached some accounts just now, and moments posted through the API.

const MOMENTS = "tokuly.moments.v1.MomentService";
const REPO = fileURLToPath(new URL("../../..", import.meta.url));

/**
 * Runs SQL on the stack's database: `TOKULY_E2E_DATABASE_URL` when set (an API of one's
 * own, e.g. `tools/agent/machine/dev.sh api-here`), otherwise the Compose stack's Postgres.
 */
function sql(query: string): string {
  const url = process.env.TOKULY_E2E_DATABASE_URL;
  const [command, args] = url
    ? ["psql", [url]]
    : ["docker", ["compose", "exec", "-T", "postgres", "psql", "-U", "tokuly", "-d", "tokuly"]];
  return execFileSync(command, [...args, "-qAtX", "-v", "ON_ERROR_STOP=1", "-c", query], {
    cwd: REPO,
    encoding: "utf8",
  }).trim();
}

/**
 * The prompt of a day reaches `accounts`, `secondsAgo` ago. The prompt time is a secret drawn at
 * random, so the tests do not wait for it: each gets a region of its own (nobody else is in it,
 * and the planner never draws it) with a day already told, on today's date in Tokyo, where the
 * test accounts live. The window is the server's, from that moment. Not the staff call
 * `SendPromptNow`: it wakes a real region once a day, before accounts made later exist.
 */
export function promptReached(accounts: Account[], secondsAgo = 0): void {
  const region = `e2e-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
  const at = `now() - make_interval(secs => ${Number(secondsAgo)})`;
  const users = accounts.map((account) => `'${account.userId}'`).join(",");
  sql(`
    INSERT INTO moment_regions (region) VALUES ('${region}');
    WITH day AS (
      INSERT INTO moment_days (region, date, prompt_at, announced_at, told_at)
      VALUES ('${region}', (now() AT TIME ZONE 'Asia/Tokyo')::date, ${at}, ${at}, now())
      RETURNING id, date
    )
    INSERT INTO moment_people (user_id, day_id, date)
    SELECT person, day.id, day.date FROM day, unnest(ARRAY[${users}]::uuid[]) AS person
    ON CONFLICT (user_id) DO UPDATE SET day_id = excluded.day_id, date = excluded.date;
    UPDATE moment_regions SET current_day_id = (SELECT id FROM moment_days WHERE region = '${region}')
    WHERE region = '${region}';
  `);
}

/** Posts `as`'s moment of the day through the API (two pictures of `color`); returns its ID. */
export async function postMoment(
  request: APIRequestContext,
  as: Account,
  {
    color = [240, 120, 40],
    caption = "",
    location,
  }: { color?: Rgb; caption?: string; location?: { lat: number; lng: number } } = {},
): Promise<string> {
  const [back, front] = await Promise.all([
    uploadPicture(request, as, discPicture("back.png", color, 256), "ASSET_PURPOSE_MOMENT"),
    uploadPicture(
      request,
      as,
      discPicture("front.png", [color[2], color[1], color[0]], 256),
      "ASSET_PURPOSE_MOMENT",
    ),
  ]);
  const { moment } = await rpc<{ moment: { id: string } }>(
    request,
    `${MOMENTS}/PostMoment`,
    { backAssetId: back, frontAssetId: front, caption, location },
    as,
  );
  return moment.id;
}
