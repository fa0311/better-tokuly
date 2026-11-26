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

// `me` (who joins) and an `owner` with a public community and one thread in its first
// channel. The suffix is unique, so a search for it finds this community only.
const me = signUp("me");
const owner = signUp("ow");
const suffix = `${Date.now().toString(36).slice(-5)}${Math.floor(Math.random() * 1000)}`;
const COMMUNITY = "tokuly.community.v1";
const created = call(
  `${COMMUNITY}.CommunityService/CreateCommunity`,
  {
    name: `ねこ部 ${suffix}`,
    handle: `e2e_${suffix}`,
    description: "Maestro のテスト用コミュニティ",
    category: "COMMUNITY_CATEGORY_CLUB",
    visibility: "COMMUNITY_VISIBILITY_PUBLIC",
  },
  owner.token,
);
const channels = call(
  `${COMMUNITY}.CommunityBoardService/ListChannels`,
  { communityId: created.community.id },
  owner.token,
);
const thread = call(
  `${COMMUNITY}.CommunityBoardService/CreateThread`,
  {
    channelId: channels.channels[0].id,
    title: `はじめまして ${suffix}`,
    body: "ねこ好き集まれ！",
  },
  owner.token,
);
output.me = me.handle;
output.owner = owner.handle;
output.suffix = suffix;
output.handle = created.community.handle;
output.thread = thread.thread.shortId;
