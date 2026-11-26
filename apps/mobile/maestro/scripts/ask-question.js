// A friend asks `me` (ME, a user ID) an anonymous question with the token TOKEN. SERVER
// comes from the command line.
function call(method, body, token) {
  const headers = { "content-type": "application/json" };
  if (token) headers.authorization = `Bearer ${token}`;
  const response = http.post(`${SERVER}/api/${method}`, { headers, body: JSON.stringify(body) });
  if (!response.ok) throw new Error(`${method}: ${response.status} ${response.body}`);
  return json(response.body);
}

const text = `e2e しつもん ${Date.now().toString(36)}`;
call("tokuly.ask.v1.QuestionBoxService/AskQuestion", { user: ME, text, signed: false }, TOKEN);
output.question = text;
