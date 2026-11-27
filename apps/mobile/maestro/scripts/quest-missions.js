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

// Missions are drawn per player and day, so the flow takes a new player whose draw holds
// one that a request can finish, and finishes it. (The day's reroll is a second chance.)
const RECIPES = {
  daily_bonus: (me) => call("tokuly.wallet.v1.WalletService/ClaimDailyBonus", {}, me.token),
  note_1: (me) => {
    const rules = call("tokuly.notes.v1.NoteService/GetNoteRules", {}, me.token);
    call("tokuly.notes.v1.NoteService/AgreeToNoteRules", { version: rules.version }, me.token);
    call(
      "tokuly.notes.v1.NoteService/CreateNote",
      { text: "ミッションやるぞ〜！", visibility: "VISIBILITY_FRIENDS" },
      me.token,
    );
  },
  add_friend: (me) => {
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
  },
};
const MISSIONS = "tokuly.quest.v1.MissionService";
const doable = (missions) => missions.daily.find((mission) => RECIPES[mission.key]);

let me;
let mission;
for (let attempt = 0; attempt < 6 && !mission; attempt++) {
  me = signUp("me");
  const drawn = call(`${MISSIONS}/ListMissions`, {}, me.token);
  mission = doable(drawn);
  if (!mission) {
    const swapped = call(`${MISSIONS}/RerollMission`, { missionId: drawn.daily[0].id }, me.token);
    if (RECIPES[swapped.mission.key]) mission = swapped.mission;
  }
}
if (!mission) throw new Error("no new player drew a mission this script can finish");
RECIPES[mission.key](me);

// Progress is counted from the events the recipe caused, a moment later (later still on a
// slow machine): the flow starts once the server has the mission done.
const done = () =>
  call(`${MISSIONS}/ListMissions`, {}, me.token).daily.some(
    (drawn) => drawn.key === mission.key && drawn.state === "MISSION_STATE_COMPLETED",
  );
const deadline = Date.now() + 60000;
while (!done()) {
  if (Date.now() > deadline) throw new Error(`${mission.key} was not counted as done`);
  // Maestro's scripts have no timers: a short pause between asks.
  for (const until = Date.now() + 300; Date.now() < until; );
}
output.me = me.handle;
output.mission = mission.key;
