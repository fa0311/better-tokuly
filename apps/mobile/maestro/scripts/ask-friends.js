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

function befriend(a, b) {
  const sent = call(
    "tokuly.social.v1.FriendService/SendFriendRequest",
    { userId: b.id, source: "FRIEND_SOURCE_HANDLE" },
    a.token,
  );
  call(
    "tokuly.social.v1.FriendService/RespondFriendRequest",
    { requestId: sent.request.id, accept: true },
    b.token,
  );
}

// `me` with three friends (a friends-only box needs three before it takes anonymous
// questions). The first friend's box is open; `me`'s own is closed, as every new box is.
const me = signUp("me");
const friend = signUp("fr");
befriend(me, friend);
befriend(me, signUp("f2"));
befriend(me, signUp("f3"));
call("tokuly.ask.v1.QuestionBoxService/UpdateSettings", { boxOpen: true }, friend.token);
output.me = me.handle;
output.meId = me.id;
output.friend = friend.handle;
output.friendToken = friend.token;
