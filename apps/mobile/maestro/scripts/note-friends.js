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

// `me` (who the app signs in as, and who has not agreed to the note rules yet) and a
// `friend`, friends already, who agreed and wrote a note for friends only: it is on my
// home timeline.
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
const rules = call("tokuly.notes.v1.NoteService/GetNoteRules", {}, friend.token);
call("tokuly.notes.v1.NoteService/AgreeToNoteRules", { version: rules.version }, friend.token);
const text = `放課後カラオケ行く人〜？ ${friend.handle.slice(-4)}`;
call(
  "tokuly.notes.v1.NoteService/CreateNote",
  { text, visibility: "VISIBILITY_FRIENDS" },
  friend.token,
);
output.me = me.handle;
output.friendNote = text;
output.reply = `行く行く！ ${me.handle.slice(-4)}`;
output.mine = `今日の帰り道 ${me.handle.slice(-4)}`;
