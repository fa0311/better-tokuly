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

// `me` watches; `creator` has a channel for `me` to find and subscribe to. (A long video
// cannot be uploaded from here: Maestro's scripts send text, not a video's bytes. Playback
// is covered by the watch screen's component tests.)
const me = signUp("me");
const creator = signUp("cr");
const channelHandle = `ch_${creator.handle}`.slice(0, 24);
const channelName = `ゆるvlog ${creator.handle.slice(-4)}`;
call(
  "tokuly.video.v1.ChannelService/CreateChannel",
  { handle: channelHandle, name: channelName },
  creator.token,
);
output.me = me.handle;
output.channelHandle = channelHandle;
output.channelName = channelName;
