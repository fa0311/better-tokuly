// The friend answers in the conversation, through the API (Connect RPC with JSON), while
// the app has it open. The token and conversation come from chat-friends.js.
const response = http.post(`${SERVER}/api/tokuly.chat.v1.MessageService/SendMessage`, {
  headers: { "content-type": "application/json", authorization: `Bearer ${output.friendToken}` },
  body: JSON.stringify({
    conversationId: output.conversation,
    clientId: `maestro-reply-${Date.now()}`,
    text: "いいね、またあとで！",
  }),
});
if (!response.ok) throw new Error(`SendMessage: ${response.status} ${response.body}`);
