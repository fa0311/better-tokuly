// Arranges state through the API (Connect RPC with JSON), the way the web's browser tests
// do. SERVER comes from the command line, PASSWORD from the flow. (Maestro scripts cannot
// import each other, so each one carries these two helpers.)
function call(method, body, token) {
  const headers = { "content-type": "application/json" };
  if (token) headers.authorization = `Bearer ${token}`;
  const response = http.post(`${SERVER}/api/${method}`, { headers, body: JSON.stringify(body) });
  if (!response.ok) throw new Error(`${method}: ${response.status} ${response.body}`);
  return json(response.body);
}

function signUp(prefix) {
  const handle = `${prefix}${Date.now().toString(36)}${Math.floor(Math.random() * 1000)}`;
  const response = call("tokuly.identity.v1.AuthService/SignUp", {
    email: `${handle}@e2e.tokuly.test`,
    password: PASSWORD,
    handle,
    displayName: `${prefix} ${handle.slice(-4)}`,
    birthdate: "2007-05-05",
    acceptTerms: true,
    client: { deviceId: `maestro-${handle}` },
  });
  return { handle, token: response.accessToken, id: response.me.user.id };
}

// A `creator` with a channel (who posts a short from the phone) and `me`, who watches the
// for-you feed: a new public short of someone else is in it.
const me = signUp("me");
const creator = signUp("cr");
call(
  "tokuly.video.v1.ChannelService/CreateChannel",
  {
    handle: `ch_${creator.handle}`.slice(0, 24),
    name: `まいにちダンス ${creator.handle.slice(-4)}`,
  },
  creator.token,
);
// The for-you feed shows the short again once nothing unseen is left: `me` has seen every
// short already on the stack (other flows' and earlier runs'), so the new one is all that is
// left to show.
const SHORTS = "tokuly.shorts.v1.ShortFeedService";
let cursor = "";
for (let page = 0; page < 100; page++) {
  const feed = call(`${SHORTS}/GetForYouFeed`, { page: { limit: 30, cursor } }, me.token);
  const items = feed.items ?? [];
  const unseen = items.filter((item) => !item.repeat);
  if (unseen.length > 0)
    call(
      `${SHORTS}/ReportPlayback`,
      {
        reports: unseen.map((item) => ({
          shortId: item.short.id,
          source: "PLAYBACK_SOURCE_FOR_YOU",
          watchedMs: 2000,
        })),
      },
      me.token,
    );
  cursor = feed.page?.nextCursor || "";
  if (unseen.length < items.length || !cursor) break;
}
output.me = me.handle;
output.creator = creator.handle;
output.caption = `maestro ${creator.handle.slice(-4)}`;
