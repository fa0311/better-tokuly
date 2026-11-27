// Arranges state through the API (Connect RPC with JSON). SERVER comes from the command
// line, PASSWORD from the flow. (Maestro scripts cannot import each other, so each one
// carries these two helpers.)
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

// `me` (who the app signs in as) and a `friend` who wrote first, and the store's first
// featured free pack (the official packs are seeded), with its first stamp.
const me = signUp("me");
const friend = signUp("fr");
const sent = call(
  "tokuly.social.v1.FriendService/SendFriendRequest",
  { userId: me.id, source: "FRIEND_SOURCE_HANDLE" },
  friend.token,
);
call(
  "tokuly.social.v1.FriendService/RespondFriendRequest",
  { requestId: sent.request.id, accept: true },
  me.token,
);
const direct = call(
  "tokuly.chat.v1.ConversationService/OpenDirect",
  { userId: me.id },
  friend.token,
);
call(
  "tokuly.chat.v1.MessageService/SendMessage",
  {
    conversationId: direct.conversation.id,
    clientId: `maestro-${Date.now()}`,
    text: "スタンプちょうだい！",
  },
  friend.token,
);
const home = call("tokuly.stamp.v1.StampStoreService/GetStoreHome", {}, me.token);
const free = home.featured.filter((pack) => !pack.price)[0];
if (!free) throw new Error("no free featured pack in the store");
const pack = call("tokuly.stamp.v1.StampStoreService/GetPack", { packId: free.id }, me.token).pack;
output.me = me.handle;
output.friendName = `fr ${friend.handle.slice(-4)}`;
output.packId = free.id;
output.stampId = pack.stamps[0].stampRef.id;
output.note = `stamp note ${Date.now().toString(36)}`;
