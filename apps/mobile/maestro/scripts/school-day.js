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

// `me` (the student) with a timetable that has 数学 first thing every day of the week (so
// today has a class whatever the weekday), an exam coming up and homework due tomorrow;
// and a `friend` who is studying right now.
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

const SCHOOL = "tokuly.school.v1";
const pad = (value) => String(value).padStart(2, "0");
const dateIn = (days) => {
  const at = new Date(Date.now() + 9 * 3_600_000 + days * 86_400_000); // Asia/Tokyo, the accounts' zone
  return `${at.getUTCFullYear()}-${pad(at.getUTCMonth() + 1)}-${pad(at.getUTCDate())}`;
};
const WEEK = ["MONDAY", "TUESDAY", "WEDNESDAY", "THURSDAY", "FRIDAY", "SATURDAY", "SUNDAY"].map(
  (day) => `WEEKDAY_${day}`,
);
const subject = call(
  `${SCHOOL}.TimetableService/CreateSubject`,
  { name: "数学", emoji: "📐", color: "#4fc3ff" },
  me.token,
);
const term = call(
  `${SCHOOL}.TimetableService/CreateTerm`,
  {
    name: "2学期",
    startDate: dateIn(-30),
    endDate: dateIn(60),
    layout: { preset: "LAYOUT_PRESET_HIGH_SCHOOL_6", weekdays: WEEK },
    observePublicHolidays: false,
    current: true,
  },
  me.token,
);
call(
  `${SCHOOL}.TimetableService/PutTimetableEntries`,
  {
    termId: term.term.id,
    entries: WEEK.map((weekday) => ({
      weekday,
      period: 1,
      subjectId: subject.subject.id,
      room: "3-A",
    })),
  },
  me.token,
);
call(
  `${SCHOOL}.ExamService/CreateExam`,
  { name: "期末テスト", startDate: dateIn(10), endDate: dateIn(14) },
  me.token,
);
const homework = call(
  `${SCHOOL}.AssignmentService/CreateAssignment`,
  {
    title: "数学のワーク p.30",
    subjectId: subject.subject.id,
    dueAt: new Date(Date.now() + 30 * 3_600_000).toISOString(),
  },
  me.token,
);
call(`${SCHOOL}.StudyService/StartStudySession`, { mode: "STUDY_MODE_STOPWATCH" }, friend.token);

output.me = me.handle;
output.friend = friend.id;
output.homework = homework.assignment.id;
