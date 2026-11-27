// The friend opens a friends room on the spot, through the API (Connect RPC with JSON); the
// accounts come from space-friends.js. The friend never connects to the room's audio, and the
// server counts someone who entered and never connected as gone after a while
// (`TOKULY_SPACES_CONNECT_GRACE_SECONDS`, 45 s): with nobody it knows inside, the room leaves
// `me`'s friends shelf. So the room is opened right before the flow looks for it, and the
// flow goes on once the server shows it to `me`.
function call(method, body, token) {
  const headers = { "content-type": "application/json", authorization: `Bearer ${token}` };
  const response = http.post(`${SERVER}/api/${method}`, { headers, body: JSON.stringify(body) });
  if (!response.ok) throw new Error(`${method}: ${response.status} ${response.body}`);
  return json(response.body);
}

const title = `e2e room ${Date.now().toString(36)}`;
call(
  "tokuly.spaces.v1.SpaceService/CreateSpace",
  { title, audience: "SPACE_AUDIENCE_FRIENDS" },
  output.friendToken,
);
const shown = () =>
  (
    call(
      "tokuly.spaces.v1.SpaceService/ListLiveSpaces",
      { scope: "LIVE_SCOPE_FRIENDS" },
      output.meToken,
    ).spaces ?? []
  ).some((space) => space.title === title);
const deadline = Date.now() + 30000;
while (!shown()) {
  if (Date.now() > deadline) throw new Error("the friend's room was not shown to me");
  // Maestro's scripts have no timers: a short pause between asks.
  for (const until = Date.now() + 300; Date.now() < until; );
}
output.title = title;
