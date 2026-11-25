//! What an API token needs in order to call each RPC.
//!
//! Every RPC of the API is listed here, one by one, with what it takes:
//! reading its domain (`Read` → `<domain>:read`), writing it (`Write` →
//! `<domain>:write`, which includes reading), scopes of other domains as
//! well (`With`: an RPC whose answer carries location needs the location
//! scope whatever package it lives in), any valid token (`Any`), or a
//! signed-in session (`Session`: what a leaked token must never do —
//! obtain or mint credentials, take over or delete the account, move money
//! out, moderate).
//!
//! The table fails closed: **an RPC that is not listed cannot be called
//! with a token at all.** Nothing is inferred from a name. Whoever adds an
//! RPC adds its line, having looked at what it reads and writes; a test in
//! `tokuly-dev` walks the API's descriptor set and fails for any RPC
//! missing here.
use Rule::{Any, Read, Session, With, Write};

/// How an RPC may be reached with an API token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Access {
    /// The token needs every one of these scopes, e.g. `["feed:read", "geo:read"]`.
    Scopes(Vec<String>),
    /// Any valid token may call it, whatever its scopes.
    AnyToken,
    /// No token may call it: it takes an interactive session.
    SessionOnly,
}

#[derive(Clone, Copy)]
enum Rule {
    /// `<domain>:read`.
    Read,
    /// `<domain>:write`.
    Write,
    /// All of the scopes named.
    With(&'static [&'static str]),
    Any,
    Session,
}

/// What a token needs in order to call the RPC at `path`
/// (`/tokuly.<domain>.v1.<Service>/<Method>`, with or without the leading
/// slash). `None` when the path is not a listed RPC: tokens are refused there.
pub fn access_for(path: &str) -> Option<Access> {
    let (service, method) = path.strip_prefix('/').unwrap_or(path).split_once('/')?;
    let (_, methods) = TABLE.iter().find(|(name, _)| *name == service)?;
    let (_, rule) = methods.iter().find(|(name, _)| *name == method)?;
    let domain = service.strip_prefix("tokuly.")?.split('.').next()?;
    Some(match rule {
        Read => Access::Scopes(vec![format!("{domain}:read")]),
        Write => Access::Scopes(vec![format!("{domain}:write")]),
        With(scopes) => Access::Scopes(scopes.iter().map(|scope| (*scope).to_owned()).collect()),
        Any => Access::AnyToken,
        Session => Access::SessionOnly,
    })
}

/// Every listed RPC as `(path, access)`, for the API reference and for tests.
pub fn classified() -> impl Iterator<Item = (String, Access)> {
    TABLE.iter().flat_map(|(service, methods)| {
        methods.iter().filter_map(move |(method, _)| {
            let path = format!("/{service}/{method}");
            access_for(&path).map(|access| (path, access))
        })
    })
}

#[rustfmt::skip]
const TABLE: &[(&str, &[(&str, Rule)])] = &[
    ("tokuly.admin.v1.ModerationService", &[
        ("ListCases", Session),
        ("GetCase", Session),
        ("GetReportedProfile", Session),
        ("ListCaseReports", Session),
        ("AssignCase", Session),
        ("AddCaseNote", Session),
        ("ResolveCase", Session),
        ("ReopenCase", Session),
        ("TakedownContent", Session),
        ("ListModerationActions", Session),
    ]),
    ("tokuly.admin.v1.UserAdminService", &[
        ("SearchUsers", Session),
        ("GetUser", Session),
        ("WarnUser", Session),
        ("SuspendUser", Session),
        ("UnsuspendUser", Session),
        ("SetUserVerified", Session),
        ("SetUserRoles", Session),
        ("RevokeUserSessions", Session),
    ]),
    ("tokuly.admin.v1.FeatureFlagService", &[
        ("ListFlags", Session),
        ("CreateFlag", Session),
        ("UpdateFlag", Session),
        ("DeleteFlag", Session),
        ("EvaluateFlags", Session),
    ]),
    ("tokuly.admin.v1.AnnouncementService", &[
        ("ListAnnouncements", Session),
        ("GetAnnouncement", Session),
        ("ListManagedAnnouncements", Session),
        ("GetManagedAnnouncement", Session),
        ("CreateAnnouncement", Session),
        ("UpdateAnnouncement", Session),
        ("PublishAnnouncement", Session),
        ("UnpublishAnnouncement", Session),
        ("DeleteAnnouncement", Session),
    ]),
    ("tokuly.admin.v1.SystemService", &[
        ("GetServerStats", Session),
        ("ListAuditLog", Session),
    ]),
    ("tokuly.admin.v1.ServerSettingsService", &[
        ("ListServerSettings", Session),
        ("UpdateServerSetting", Session),
        ("ExportServerSettings", Session),
        ("ListServerSettingChanges", Session),
    ]),
    ("tokuly.ask.v1.QuestionBoxService", &[
        ("GetSettings", Read),
        ("UpdateSettings", Write),
        ("GetBox", Read),
        ("AskQuestion", Write),
        ("ListAskedQuestions", Read),
        ("WithdrawQuestion", Write),
        ("ListInbox", Read),
        ("GetInboxCounts", Read),
        ("MarkQuestionsRead", Write),
        ("MoveQuestion", Write),
        ("DeleteQuestion", Write),
        ("AnswerQuestion", Write),
        ("UpdateAnswer", Write),
        ("DeleteAnswer", Write),
        ("ListAnswers", Read),
        ("GetQuestion", Read),
        ("BlockSender", Write),
        ("ListSenderBlocks", Read),
        ("UnblockSender", Write),
    ]),
    ("tokuly.ask.v1.PromptService", &[
        ("CreatePrompt", Write),
        ("ClosePrompt", Write),
        ("DeletePrompt", Write),
        ("GetPrompt", Read),
        ("ListPrompts", Read),
        ("ListFriendPrompts", Read),
        ("RespondToPrompt", Write),
        ("ListPromptResponses", Read),
        ("ListPublishedResponses", Read),
        ("SetResponsePublished", Write),
        ("ReleaseResponse", Write),
        ("DeleteResponse", Write),
    ]),
    ("tokuly.ask.v1.ComplimentService", &[
        ("GetRound", Read),
        // Deliberately without the wallet scope: finishing a round pays its small
        // reward into the caller's own account, and nothing can be spent here.
        ("Vote", Write),
        ("SkipPoll", Write),
        ("ShufflePoll", Write),
        ("ListFlames", Read),
        ("GetFlameSummary", Read),
        ("MarkFlamesSeen", Write),
        ("BuyHint", With(&["ask:write", "wallet:write"])),
        ("GetTopFlames", Read),
    ]),
    ("tokuly.ask.v1.AskModerationService", &[
        ("InspectQuestion", Session),
        ("InspectPromptResponse", Session),
        ("InspectFlame", Session),
        ("ListPollQuestions", Session),
        ("AddPollQuestion", Session),
        ("SetPollQuestionEnabled", Session),
    ]),
    ("tokuly.chat.v1.ConversationService", &[
        ("GetCapabilities", Read),
        ("OpenDirect", Write),
        ("CreateGroup", Write),
        ("GetConversation", Read),
        ("ListConversations", Read),
        ("UpdateGroup", Write),
        ("ListMembers", Read),
        ("AddMembers", Write),
        ("RemoveMember", Write),
        ("SetMemberRole", Write),
        ("LeaveConversation", Write),
        ("UpdatePreferences", Write),
        ("HideConversation", Write),
        ("RespondToRequest", Write),
        ("MarkRead", Write),
        ("GetUnreadCounts", Read),
    ]),
    ("tokuly.chat.v1.MessageService", &[
        ("SendMessage", Write),
        ("SendHello", Write),
        ("EditMessage", Write),
        ("DeleteMessage", Write),
        ("GetMessage", Read),
        ("ListMessages", Read),
        ("SearchMessages", Read),
        ("ForwardMessages", Write),
        ("AddReaction", Write),
        ("RemoveReaction", Write),
        ("ListReactions", Read),
        ("PinMessage", Write),
        ("UnpinMessage", Write),
        ("ListPinnedMessages", Read),
        ("ListReadReceipts", Read),
        ("ListSharedMedia", Read),
    ]),
    ("tokuly.chat.v1.PovService", &[
        ("RequestPov", Write),
        ("AnswerPov", Write),
        ("DeclinePov", Write),
        ("CancelPov", Write),
        ("SavePov", Write),
    ]),
    ("tokuly.chat.v1.CallService", &[
        ("StartCall", Write),
        ("JoinCall", Write),
        ("DeclineCall", Write),
        ("LeaveCall", Write),
        ("GetCall", Read),
        ("ListCalls", Read),
    ]),
    ("tokuly.chat.v1.AssistService", &[
        ("RewriteDraft", Write),
        ("SuggestReplies", Write),
        ("TranslateMessage", Write),
        ("GetAssistSettings", Read),
        ("ChooseAssistProvider", Write),
        // Storing a credential the server then spends on the user's behalf.
        ("SetOwnApiKey", Session),
        ("RemoveOwnApiKey", Session),
        ("SetAiMessageConsent", Session),
    ]),
    ("tokuly.chat.v1.AssistAdminService", &[
        ("GetAssistInstance", Session),
        ("UpdateAssistProvider", Session),
        ("SetDefaultAssistProvider", Session),
    ]),
    ("tokuly.dev.v1.TokenService", &[
        ("ListScopes", Any),
        ("CreateToken", Session),
        ("ListTokens", Session),
        ("RevokeToken", Session),
        ("GetCurrentToken", Any),
    ]),
    ("tokuly.dev.v1.AppService", &[
        ("CreateApp", Session),
        ("UpdateApp", Session),
        ("GetApp", Session),
        ("ListApps", Session),
        ("DeleteApp", Session),
        ("RotateClientSecret", Session),
        ("ListAppUsers", Session),
    ]),
    ("tokuly.dev.v1.AuthorizationService", &[
        ("GetPublicApp", Session),
        ("GetConsent", Session),
        ("ApproveAuthorization", Session),
        ("DenyAuthorization", Session),
        ("GetDeviceConsent", Session),
        ("ApproveDevice", Session),
        ("DenyDevice", Session),
        ("ListAuthorizedApps", Session),
        ("RevokeAuthorizedApp", Session),
    ]),
    ("tokuly.dev.v1.BotService", &[
        ("CreateBot", Session),
        ("GetBot", Session),
        ("ListBots", Session),
        ("UpdateBot", Session),
        ("SetBotDisabled", Session),
        ("DeleteBot", Session),
        ("CreateBotToken", Session),
        ("ListBotTokens", Session),
        ("RevokeBotToken", Session),
        // Handing an account over is not for a token, as minting its credentials is not.
        ("OfferBotTransfer", Session),
        ("ListBotTransfers", Session),
        ("AcceptBotTransfer", Session),
        ("CancelBotTransfer", Session),
    ]),
    ("tokuly.dev.v1.WebhookService", &[
        ("ListEventTypes", Any),
        ("CreateWebhook", Session),
        ("UpdateWebhook", Session),
        ("GetWebhook", Read),
        ("ListWebhooks", Read),
        ("DeleteWebhook", Session),
        ("RotateWebhookSecret", Session),
        ("PingWebhook", Session),
        ("ListDeliveries", Read),
        ("GetDelivery", Read),
        ("ReplayDelivery", Session),
    ]),
    ("tokuly.dev.v1.ReferenceService", &[
        ("ListServices", Any),
        ("GetMethod", Any),
    ]),
    ("tokuly.dev.v1.UsageService", &[
        ("GetUsage", Read),
    ]),
    ("tokuly.emoji.v1.EmojiService", &[
        ("GetCatalog", Read),
        ("GetCatalogChanges", Read),
        ("SearchEmoji", Read),
        ("ListPopularEmoji", Read),
        ("ListNewestEmoji", Read),
        ("GetEmoji", Read),
        ("ListCategories", Read),
        ("ListAuthorEmoji", Read),
        ("GetDeck", Read),
        ("SetDeck", Write),
        ("ResetDeck", Write),
        ("RecordUse", Write),
        ("ListRecentEmoji", Read),
        ("ClearRecentEmoji", Write),
    ]),
    ("tokuly.emoji.v1.EmojiSubmissionService", &[
        ("GetSubmitterProfile", Read),
        ("CheckName", Read),
        ("SubmitEmoji", Write),
        ("UpdateSubmission", Write),
        ("WithdrawSubmission", Write),
        ("ListMySubmissions", Read),
    ]),
    ("tokuly.emoji.v1.EmojiAdminService", &[
        ("ListReviewQueue", Session),
        ("ApproveEmoji", Session),
        ("RejectEmoji", Session),
        ("ListEmoji", Session),
        ("AddEmoji", Session),
        ("UpdateEmoji", Session),
        ("DisableEmoji", Session),
        ("EnableEmoji", Session),
        ("DeleteEmoji", Session),
        ("BulkUpdateEmoji", Session),
        ("BulkDeleteEmoji", Session),
        ("SetSubmitterTrusted", Session),
        ("ListTrustedSubmitters", Session),
        ("CreateCategory", Session),
        ("RenameCategory", Session),
        ("ReorderCategories", Session),
        ("DeleteCategory", Session),
        ("StartImport", Session),
        ("GetImport", Session),
        ("ListImports", Session),
        ("StartExport", Session),
        ("GetExport", Session),
        ("ListExports", Session),
    ]),
    ("tokuly.engagement.v1.CommentService", &[
        ("ListComments", Read),
        ("ListReplies", Read),
        ("GetComment", Read),
        ("CreateComment", Write),
        ("UpdateComment", Write),
        ("DeleteComment", Write),
        ("PinComment", Write),
        ("UnpinComment", Write),
        ("SetCommentHeart", Write),
        ("ListMyComments", Read),
    ]),
    ("tokuly.engagement.v1.CommentModerationService", &[
        ("ListReceivedComments", Read),
        ("ApproveComment", Write),
        ("HideComment", Write),
        ("GetModerationSettings", Read),
        ("UpdateModerationSettings", Write),
        ("BlockCommenter", Write),
        ("UnblockCommenter", Write),
        ("ListBlockedCommenters", Read),
    ]),
    ("tokuly.engagement.v1.ReactionService", &[
        ("SetReaction", Write),
        ("RemoveReaction", Write),
        ("ListReactions", Read),
        ("ListMyReactions", Read),
        ("GetReactionSettings", Read),
        ("UpdateReactionSettings", Write),
    ]),
    ("tokuly.engagement.v1.BookmarkService", &[
        ("AddBookmark", Write),
        ("RemoveBookmark", Write),
        ("ListBookmarks", Read),
        ("ListCollections", Read),
        ("CreateCollection", Write),
        ("UpdateCollection", Write),
        ("DeleteCollection", Write),
    ]),
    ("tokuly.engagement.v1.EngagementService", &[
        ("GetEngagement", Read),
        ("BatchGetEngagement", Read),
        ("RecordShare", Write),
        ("GetShareStats", Read),
    ]),
    ("tokuly.engagement.v1.ReportService", &[
        ("CreateReport", Write),
        // A report about a chat message quotes it.
        ("ListMyReports", With(&["engagement:read", "chat:read"])),
    ]),
    ("tokuly.feed.v1.ActivityService", &[
        ("ListMyActivity", With(&["feed:read", "geo:read"])),
        ("DeleteActivity", Write),
        ("ClearActivity", Write),
        ("ListUserActivity", With(&["feed:read", "geo:read"])),
        ("ListFriendsActivity", With(&["feed:read", "geo:read"])),
        ("GetActivityPreferences", Read),
        ("SetActivityKindVisibility", Write),
    ]),
    ("tokuly.feed.v1.MapStoryService", &[
        ("ListMapStories", With(&["feed:read", "geo:read"])),
        ("GetMapStoryTray", With(&["feed:read", "geo:read"])),
        ("MarkMapStorySeen", Write),
    ]),
    ("tokuly.feed.v1.HomeFeedService", &[
        ("ListHomeFeed", With(&["feed:read", "geo:read"])),
    ]),
    ("tokuly.feed.v1.NewsService", &[
        ("ListNews", Read),
        ("GetNewsArticle", Read),
        ("RecordNewsClick", Write),
        ("SendNewsFeedback", Write),
        ("ListNewsTags", Read),
        ("ResetNewsInterests", Write),
        ("ListNewsSources", Read),
    ]),
    ("tokuly.feed.v1.NewsAdminService", &[
        ("ListManagedNewsSources", Session),
        ("CreateNewsSource", Session),
        ("UpdateNewsSource", Session),
        ("DeleteNewsSource", Session),
        ("RefreshNewsSources", Session),
        ("RemoveNewsArticle", Session),
    ]),
    ("tokuly.feed.v1.RecapService", &[
        ("GetRecap", With(&["feed:read", "geo:read"])),
    ]),
    ("tokuly.geo.v1.LocationService", &[
        ("ReportLocations", Write),
        ("StopSharing", Write),
        ("ListFriendLocations", Read),
        ("WatchFriendLocations", Read),
        ("GetMyLocation", Read),
        ("GetGhostSettings", Read),
        ("SetGhostMode", Write),
        ("ListGatherings", Read),
        ("CreateLiveShare", Write),
        ("ListLiveShares", Read),
        ("RevokeLiveShare", Write),
        ("GetLiveShare", Read),
        ("WatchLiveShare", Read),
    ]),
    ("tokuly.geo.v1.PlaceService", &[
        ("ListMyPlaces", Read),
        ("CreatePlace", Write),
        ("UpdatePlace", Write),
        ("DeletePlace", Write),
        ("GetPlace", Read),
        ("SearchPlaces", Read),
        ("ListPlaceSuggestions", Read),
        ("SavePlace", Write),
        ("UnsavePlace", Write),
        ("ListSavedPlaces", Read),
        ("ReverseGeocode", Read),
        ("CheckIn", Write),
    ]),
    ("tokuly.geo.v1.HistoryService", &[
        ("ListVisits", Read),
        ("ListActiveDays", Read),
        ("GetDay", Read),
        ("GetRoute", Read),
        ("SetDayMemo", Write),
        ("ListNights", Read),
        ("GetStats", Read),
        ("DeleteHistory", Write),
    ]),
    ("tokuly.geo.v1.FootprintService", &[
        ("GetCells", Read),
        ("GetStats", Read),
        ("GetLeaderboard", Read),
        ("GetRecap", Read),
    ]),
    ("tokuly.geo.v1.TripService", &[
        ("StartTrip", Write),
        ("EndTrip", Write),
        ("GetTrip", Read),
        ("ListActiveTrips", Read),
        ("GetDirections", Read),
    ]),
    ("tokuly.geo.v1.SafetyService", &[
        ("CreatePlaceAlert", Write),
        ("RespondPlaceAlert", Write),
        ("ListPlaceAlerts", Read),
        ("DeletePlaceAlert", Write),
        ("TriggerSos", Session),
        ("ResolveSos", Session),
        ("GetActiveSos", Read),
    ]),
    ("tokuly.identity.v1.AuthService", &[
        ("SignUp", Session),
        ("SignIn", Session),
        ("RefreshToken", Session),
        ("SignOut", Session),
        ("CheckHandle", Session),
        // Says only what the server supports; nothing about anyone.
        ("GetAuthCapabilities", Any),
        ("BeginPasskeyRegistration", Session),
        ("FinishPasskeyRegistration", Session),
        ("BeginPasskeySignIn", Session),
        ("FinishPasskeySignIn", Session),
        ("RequestEmailVerification", Session),
        ("VerifyEmail", Session),
        ("RequestPasswordReset", Session),
        ("ResetPassword", Session),
        ("StartDeviceLogin", Session),
        ("PollDeviceLogin", Session),
        ("GetDeviceLogin", Session),
        ("ApproveDeviceLogin", Session),
        // Holding and switching the accounts of a device is credential handling.
        ("ListDeviceAccounts", Session),
        ("SwitchAccount", Session),
        ("RemoveDeviceAccount", Session),
    ]),
    ("tokuly.identity.v1.AccountService", &[
        ("GetMe", Read),
        ("UpdateProfile", Write),
        ("SetStatus", Write),
        ("GetSettings", Read),
        ("UpdateSettings", Write),
        ("ChangePassword", Session),
        ("ChangeEmail", Session),
        ("ListSessions", Session),
        ("RevokeSession", Session),
        ("ListPasskeys", Session),
        ("DeletePasskey", Session),
        ("BeginTotpSetup", Session),
        ("EnableTotp", Session),
        ("DisableTotp", Session),
        ("ListSecurityEvents", Session),
        ("RequestAccountDeletion", Session),
        ("CancelAccountDeletion", Session),
        ("RequestDataExport", Session),
        ("ListConnections", Session),
        ("BeginConnection", Session),
        ("FinishConnection", Session),
        ("DeleteConnection", Session),
        ("UpdateProfileExtras", Write),
        ("PinSong", Write),
        ("RecheckProfileLinks", Write),
    ]),
    ("tokuly.identity.v1.ProfileService", &[
        ("GetProfile", With(&["identity:read", "geo:read"])),
        ("BatchGetUsers", Read),
        ("SearchUsers", Read),
        ("RecordProfileView", Write),
        ("GetProfileViews", Read),
        ("ListProfilePrompts", Read),
        ("SuggestProfileTags", Read),
    ]),
    ("tokuly.media.v1.MediaService", &[
        ("CreateUpload", Write),
        ("GetUploadParts", Write),
        ("CompleteUpload", Write),
        ("AbortUpload", Write),
        ("GetAsset", Read),
        ("BatchGetAssets", Read),
        ("WatchAsset", Read),
        ("DeleteAsset", Write),
        ("GetLinkPreview", Read),
    ]),
    ("tokuly.notes.v1.NoteService", &[
        ("GetNoteRules", Read),
        ("AgreeToNoteRules", Write),
        ("CreateNote", Write),
        ("DeleteNote", Write),
        ("SetNoteVisibility", Write),
        ("PinNote", Write),
        ("UnpinNote", Write),
        ("GetNote", Read),
        ("BatchGetNotes", Read),
        ("GetNoteThread", Read),
        ("ListNoteQuotes", Read),
        ("RecordNoteViews", Write),
        ("LikeNote", Write),
        ("UnlikeNote", Write),
        ("RenoteNote", Write),
        ("UndoRenote", Write),
        ("ListNoteLikers", Read),
        ("ListNoteRenoters", Read),
        ("GetHomeTimeline", Read),
        ("GetFollowingTimeline", Read),
        ("GetGlobalTimeline", Read),
        ("GetUserTimeline", Read),
        ("GetHashtagTimeline", Read),
        ("GetMentionsTimeline", Read),
        ("ListTrendingHashtags", Read),
        ("SearchHashtags", Read),
        ("ListTrendingNotes", Read),
        ("EditNote", Write),
        ("ListNoteEdits", Read),
        ("RedraftNote", Write),
        ("VotePoll", Write),
        ("GetNoteAnalytics", Read),
    ]),
    ("tokuly.notes.v1.NoteDraftService", &[
        ("SaveDraft", Write),
        ("GetDraft", Read),
        ("ListDrafts", Read),
        ("DeleteDraft", Write),
        ("PublishDraft", Write),
    ]),
    ("tokuly.notes.v1.NoteMuteService", &[
        ("GetMuteSettings", Read),
        ("UpdateMutedWords", Write),
        ("MuteThread", Write),
        ("UnmuteThread", Write),
        ("HideRenotesFrom", Write),
        ("ShowRenotesFrom", Write),
    ]),
    ("tokuly.notes.v1.NoteListService", &[
        ("CreateList", Write),
        ("UpdateList", Write),
        ("DeleteList", Write),
        ("GetList", Read),
        ("ListLists", Read),
        ("ListFollowedLists", Read),
        ("ListListsContainingMe", Read),
        ("AddListMembers", Write),
        ("RemoveListMembers", Write),
        ("ListListMembers", Read),
        ("LeaveList", Write),
        ("FollowList", Write),
        ("UnfollowList", Write),
        ("GetListTimeline", Read),
    ]),
    ("tokuly.notes.v1.AntennaService", &[
        ("CreateAntenna", Write),
        ("UpdateAntenna", Write),
        ("DeleteAntenna", Write),
        ("GetAntenna", Read),
        ("ListAntennas", Read),
        ("GetAntennaTimeline", Read),
    ]),
    ("tokuly.notes.v1.NoteChannelService", &[
        ("CreateChannel", Write),
        ("UpdateChannel", Write),
        ("ArchiveChannel", Write),
        ("GetChannel", Read),
        ("ListChannels", Read),
        ("FollowChannel", Write),
        ("UnfollowChannel", Write),
        ("AddChannelModerator", Write),
        ("RemoveChannelModerator", Write),
        ("ListChannelModerators", Read),
        ("PinChannelNote", Write),
        ("UnpinChannelNote", Write),
        ("RemoveChannelNote", Write),
        ("GetChannelTimeline", Read),
    ]),
    ("tokuly.notes.v1.ClipService", &[
        ("CreateClip", Write),
        ("UpdateClip", Write),
        ("DeleteClip", Write),
        ("GetClip", Read),
        ("ListClips", Read),
        ("AddNoteToClip", Write),
        ("RemoveNoteFromClip", Write),
        ("GetClipNotes", Read),
        ("ListNoteClips", Read),
    ]),
    ("tokuly.notes.v1.StoryService", &[
        ("CreateStory", Write),
        ("DeleteStory", Write),
        ("GetStory", Read),
        ("GetStoryTray", Read),
        ("ListUserStories", Read),
        ("MarkStoryViewed", Write),
        ("ListStoryViewers", Read),
        ("ReactToStory", Write),
        ("RemoveStoryReaction", Write),
        ("GetStoryCalendar", Read),
        ("ListArchivedStories", Read),
        ("CreateHighlight", Write),
        ("UpdateHighlight", Write),
        ("DeleteHighlight", Write),
        ("ReorderHighlights", Write),
        ("ListHighlights", Read),
        ("GetHighlight", Read),
    ]),
    ("tokuly.notify.v1.InboxService", &[
        ("ListNotifications", With(&["notify:read", "chat:read", "geo:read"])),
        ("GetUnreadCount", Read),
        ("MarkRead", Write),
        ("DeleteNotifications", Write),
        ("ClearNotifications", Write),
    ]),
    ("tokuly.notify.v1.PreferencesService", &[
        ("GetPreferences", Read),
        ("UpdatePreferences", Write),
        ("PausePush", Write),
        ("ResumePush", Write),
        ("MuteUser", Write),
        ("UnmuteUser", Write),
        ("ListMutedUsers", Read),
    ]),
    ("tokuly.notify.v1.PushService", &[
        ("GetPushConfig", Read),
        ("RegisterDevice", Session),
        ("UnregisterDevice", Session),
        ("ListDevices", Session),
        ("SendTestPush", Write),
    ]),
    ("tokuly.realtime.v1.RealtimeService", &[
        ("GetEndpoints", Read),
    ]),
    ("tokuly.school.v1.SchoolService", &[
        ("GetSchoolSettings", Read),
        ("UpdateSchoolSettings", Write),
    ]),
    ("tokuly.school.v1.TimetableService", &[
        ("ListLayoutPresets", Read),
        ("ListSubjects", Read),
        ("CreateSubject", Write),
        ("UpdateSubject", Write),
        ("DeleteSubject", Write),
        ("ListTerms", Read),
        ("GetTerm", Read),
        ("CreateTerm", Write),
        ("UpdateTerm", Write),
        ("DeleteTerm", Write),
        ("PutTimetableEntries", Write),
        ("DeleteTimetableEntries", Write),
        ("CreateTimetableChange", Write),
        ("DeleteTimetableChange", Write),
        ("CreateDayOverride", Write),
        ("DeleteDayOverride", Write),
        ("GetDay", Read),
        ("ListDays", Read),
        ("GetNow", Read),
        ("ListFriendTimetables", Read),
        ("GetFriendTimetable", Read),
        ("CopyFriendTimetable", Write),
        ("FindCommonFreePeriods", Read),
    ]),
    ("tokuly.school.v1.StudyService", &[
        ("StartStudySession", Write),
        ("PauseStudySession", Write),
        ("ResumeStudySession", Write),
        ("StopStudySession", Write),
        ("HeartbeatStudySession", Write),
        ("SkipStudyPhase", Write),
        ("GetCurrentStudySession", Read),
        ("LogStudySession", Write),
        ("UpdateStudySession", Write),
        ("DeleteStudySession", Write),
        ("ListStudySessions", Read),
        ("GetStudySummary", Read),
        ("GetStudyProgress", Read),
        ("GetStudyHeatmap", Read),
        ("ListStudyingFriends", Read),
        ("GetStudyRanking", Read),
        ("CheerStudy", Write),
        ("ListStudyCheers", Read),
    ]),
    ("tokuly.school.v1.StudyRoomService", &[
        ("CreateStudyRoom", Write),
        ("GetStudyRoom", Read),
        ("UpdateStudyRoom", Write),
        ("CloseStudyRoom", Write),
        ("ListStudyRooms", Read),
        ("JoinStudyRoom", Write),
        ("LeaveStudyRoom", Write),
        ("SetStudyRoomSubject", Write),
        ("ControlStudyRoomTimer", Write),
        ("SendStudyRoomReaction", Write),
        ("InviteToStudyRoom", Write),
        ("UninviteFromStudyRoom", Write),
        ("RemoveStudyRoomMember", Write),
    ]),
    ("tokuly.school.v1.ExamService", &[
        ("ListExams", Read),
        ("GetExam", Read),
        ("CreateExam", Write),
        ("UpdateExam", Write),
        ("DeleteExam", Write),
        ("PutExamSubject", Write),
        ("DeleteExamSubject", Write),
        ("SetExamResult", Write),
    ]),
    ("tokuly.school.v1.AssignmentService", &[
        ("ListAssignments", Read),
        ("CreateAssignment", Write),
        ("UpdateAssignment", Write),
        ("DeleteAssignment", Write),
    ]),
    ("tokuly.search.v1.SearchService", &[
        ("Search", Read),
        ("Suggest", Read),
        ("GetTrending", Read),
        ("ListRecentSearches", Read),
        ("DeleteRecentSearch", Write),
        ("ClearRecentSearches", Write),
        ("SetHistoryPaused", Write),
    ]),
    ("tokuly.search.v1.SearchAdminService", &[
        ("GetIndexStats", Session),
        ("Reindex", Session),
        ("DeleteDocument", Session),
        ("ListBlockedTerms", Session),
        ("AddBlockedTerm", Session),
        ("RemoveBlockedTerm", Session),
        ("ListSynonymGroups", Session),
        ("SaveSynonymGroup", Session),
        ("DeleteSynonymGroup", Session),
    ]),
    ("tokuly.social.v1.FriendService", &[
        ("SendFriendRequest", Write),
        ("CancelFriendRequest", Write),
        ("RespondFriendRequest", Write),
        ("ListFriendRequests", Read),
        ("ListFriends", Read),
        ("RemoveFriend", Write),
        ("GetFriendship", Read),
        ("UpdateFriendPrefs", Write),
        ("ListBestFriends", Read),
        ("ListSuggestions", Read),
        ("DismissSuggestion", Write),
        ("MatchContacts", Write),
        ("BlockUser", Write),
        ("UnblockUser", Write),
        ("ListBlockedUsers", Read),
        ("GetMyInvite", Read),
        ("ResetMyInvite", Write),
        ("ResolveInvite", Read),
        ("ListIntros", Read),
        ("DiscoverNearby", With(&["social:write", "geo:read"])),
    ]),
    ("tokuly.social.v1.InteractionService", &[
        ("Bump", With(&["social:write", "geo:write"])),
        ("ListBumps", With(&["social:read", "geo:read"])),
        ("Buzz", Write),
        ("Knock", Write),
        ("SendMapReaction", With(&["social:write", "geo:read"])),
        ("ListPendingMapReactions", With(&["social:read", "geo:read"])),
        ("AckMapReactions", Write),
        ("RecordMapView", With(&["social:write", "geo:read"])),
        ("GetMapViews", With(&["social:read", "geo:read"])),
    ]),
    ("tokuly.social.v1.FollowService", &[
        ("Follow", Write),
        ("Unfollow", Write),
        ("RemoveFollower", Write),
        ("ListFollowRequests", Read),
        ("RespondFollowRequest", Write),
        ("CancelFollowRequest", Write),
        ("ListFollows", Read),
        ("GetFollowSummary", Read),
        ("UpdateFollowPrefs", Write),
        ("GetFollowSettings", Read),
        ("UpdateFollowSettings", Write),
        ("ListFollowSuggestions", Read),
        ("DismissFollowSuggestion", Write),
    ]),
    ("tokuly.social.v1.CircleService", &[
        ("ListCircles", Read),
        ("GetCircle", Read),
        ("CreateCircle", Write),
        ("UpdateCircle", Write),
        ("DeleteCircle", Write),
        ("ReorderCircles", Write),
        ("AddCircleMembers", Write),
        ("RemoveCircleMembers", Write),
        ("ListCircleMembers", Read),
        ("ListCircleCandidates", Read),
    ]),
    ("tokuly.stamp.v1.StampStoreService", &[
        ("GetStoreHome", Read),
        ("ListStorePacks", Read),
        ("SearchPacks", Read),
        ("ListPopularTags", Read),
        ("ListCreatorPacks", Read),
        ("GetPack", Read),
        ("GetStamp", Read),
    ]),
    ("tokuly.stamp.v1.StampLibraryService", &[
        ("GetLibrary", Read),
        ("ListInstalledPacks", Read),
        ("InstallPack", Write),
        ("UninstallPack", Write),
        ("ReorderInstalledPacks", Write),
        // Coins leave the account: spending takes the wallet scope, whatever is bought.
        ("PurchasePack", With(&["stamp:write", "wallet:write"])),
        ("ListPurchasedPacks", Read),
        ("ListFavoriteStamps", Read),
        ("FavoriteStamp", Write),
        ("UnfavoriteStamp", Write),
        ("ListRecentStamps", Read),
        ("ClearRecentStamps", Write),
        ("SaveStamp", Write),
        ("CreatePersonalStamp", Write),
        ("ListPersonalStamps", Read),
        ("UpdatePersonalStamp", Write),
        ("DeletePersonalStamp", Write),
        ("ReorderPersonalStamps", Write),
        ("SearchLibraryStamps", Read),
        ("ListUsageLog", Read),
    ]),
    ("tokuly.stamp.v1.StampStudioService", &[
        ("GetCreatorProfile", Read),
        ("ListMyPacks", Read),
        ("CreatePack", Write),
        ("UpdatePack", Write),
        ("DeletePack", Write),
        ("AddStamps", Write),
        ("UpdateStamp", Write),
        ("RemoveStamp", Write),
        ("ReorderStamps", Write),
        ("SubmitPack", Write),
        ("UnpublishPack", Write),
        ("GetPackStats", Read),
    ]),
    ("tokuly.stamp.v1.StampReviewService", &[
        ("ListReviewQueue", Session),
        ("ApprovePack", Session),
        ("RejectPack", Session),
        ("SetCreatorTrusted", Session),
        ("SetPackFeatured", Session),
    ]),
    ("tokuly.live.v1.LiveStreamService", &[
        ("CreateStream", Write),
        ("UpdateStream", Write),
        ("DeleteStream", Write),
        // The page without a playback token: tokens of restricted streams go to sessions only.
        ("GetStream", Read),
        ("ListChannelStreams", Read),
        ("ListLiveStreams", Read),
        ("ListUpcomingStreams", Read),
        ("ListSubscribedStreams", Read),
        // Which friends are on air, friends-only streams included.
        ("ListFriendStreams", With(&["live:read", "social:read"])),
        // The stream key and the ingest URLs that carry it.
        ("GetIngestInfo", Session),
        ("RotateStreamKey", Session),
        // Going on air.
        ("StartStream", Session),
        ("EndStream", Write),
        ("SetReminder", Write),
        // Puts the account into the audience; the answer carries no playback token.
        ("ReportWatching", Write),
        // Who is watching.
        ("ListViewers", Read),
        // Ends the stream and sends its whole audience to someone else's.
        ("RaidStream", Session),
    ]),
    ("tokuly.live.v1.LiveChatService", &[
        ("SendChatMessage", Write),
        ("ListChatMessages", Read),
        ("ListChatReplay", Read),
        ("DeleteChatMessage", Write),
        ("PinChatMessage", Write),
        ("UnpinChatMessage", Write),
        ("UpdateChatSettings", Write),
        ("GetBannedWords", Read),
        ("SetBannedWords", Write),
        ("RestrictChatUser", Write),
        ("UnrestrictChatUser", Write),
        ("ListChatRestrictions", Read),
        // Hands moderation power to another account, lastingly.
        ("AddModerator", Session),
        ("RemoveModerator", Session),
        ("ListModerators", Read),
    ]),
    ("tokuly.live.v1.LivePollService", &[
        ("CreatePoll", Write),
        ("VotePoll", Write),
        ("EndPoll", Write),
        ("ListPolls", Read),
    ]),
    ("tokuly.live.v1.LiveGiftService", &[
        ("GetGiftTiers", Read),
        // Spends coins.
        ("SendGift", With(&["live:write", "wallet:write"])),
        // What the account spent and earned, and who paid it.
        ("ListSentGifts", With(&["live:read", "wallet:read"])),
        ("ListReceivedGifts", With(&["live:read", "wallet:read"])),
        ("ListTopSupporters", Read),
    ]),
    ("tokuly.live.v1.ClipService", &[
        ("CreateClip", Write),
        ("GetClip", Read),
        ("UpdateClip", Write),
        ("DeleteClip", Write),
        ("ListClips", Read),
    ]),
    ("tokuly.live.v1.OverlayService", &[
        ("ListOverlayPresets", Read),
        // A widget's token is a secret (it opens the widget in OBS without a
        // sign-in), and these return it or replace it.
        ("ListOverlays", Session),
        ("CreateOverlay", Session),
        ("UpdateOverlay", Session),
        ("DeleteOverlay", Session),
        ("RotateOverlayToken", Session),
        // Opened with the widget's own token, which the request carries.
        ("GetOverlay", Read),
        ("SendTestAlert", Write),
    ]),
    ("tokuly.live.v1.LiveAnalyticsService", &[
        ("GetRealtimeStats", Read),
        ("GetStreamAnalytics", Read),
        ("ListStreamReports", Read),
    ]),
    ("tokuly.video.v1.ChannelService", &[
        ("CreateChannel", Write),
        ("UpdateChannel", Write),
        ("DeleteChannel", Write),
        ("ListMyChannels", Read),
        ("SetDefaultChannel", Write),
        ("CheckChannelHandle", Read),
        ("GetChannel", Read),
        ("GetChannelPage", Read),
        ("ListChannelVideos", Read),
        ("Subscribe", Write),
        ("Unsubscribe", Write),
        ("ListSubscriptions", Read),
        ("ListSubscribers", Read),
        ("ListRecommendedChannels", Read),
    ]),
    ("tokuly.video.v1.VideoService", &[
        ("CreateVideo", Write),
        ("UpdateVideo", Write),
        ("PublishVideo", Write),
        ("UnpublishVideo", Write),
        ("DeleteVideo", Write),
        ("GetVideo", Read),
        ("BatchGetVideos", Read),
        ("ListStudioVideos", Read),
        ("RateVideo", Write),
        ("SuggestGames", Read),
        ("AddSubtitle", Write),
        ("UpdateSubtitle", Write),
        ("DeleteSubtitle", Write),
        ("ListSubtitles", Read),
    ]),
    ("tokuly.video.v1.PlaybackService", &[
        ("StartPlayback", Write),
        ("ReportPlayback", Write),
        ("ListWatchHistory", Read),
        ("RemoveWatchHistoryItem", Write),
        ("ClearWatchHistory", Write),
        ("GetWatchHistorySettings", Read),
        ("SetWatchHistoryPaused", Write),
    ]),
    ("tokuly.video.v1.PlaylistService", &[
        ("CreatePlaylist", Write),
        ("UpdatePlaylist", Write),
        ("DeletePlaylist", Write),
        ("GetPlaylist", Read),
        ("ListMyPlaylists", Read),
        ("ListChannelPlaylists", Read),
        ("ListPlaylistItems", Read),
        ("AddPlaylistItem", Write),
        ("RemovePlaylistItem", Write),
        ("MovePlaylistItem", Write),
    ]),
    ("tokuly.video.v1.VideoFeedService", &[
        ("GetHomeFeed", Read),
        ("GetSubscriptionsFeed", Read),
        ("ListTrending", Read),
        ("ListRelatedVideos", Read),
        ("MarkNotInterested", Write),
        ("UndoNotInterested", Write),
    ]),
    ("tokuly.video.v1.WatchPartyService", &[
        ("CreateParty", Write),
        ("PreviewParty", Read),
        ("JoinParty", Write),
        ("GetParty", Read),
        ("HeartbeatParty", Write),
        ("LeaveParty", Write),
        ("SetPartyPlayback", Write),
        ("SetPartyVideo", Write),
        ("SetPartyMemberRole", Write),
        ("RemovePartyMember", Write),
        ("UpdateParty", Write),
        ("SendPartyMessage", Write),
        ("InviteToParty", Write),
        ("EndParty", Write),
    ]),
    ("tokuly.video.v1.StudioAnalyticsService", &[
        ("GetChannelAnalytics", Read),
        ("GetVideoAnalytics", Read),
        ("GetRealtimeAnalytics", Read),
    ]),
    ("tokuly.wallet.v1.WalletService", &[
        ("GetBalance", Read),
        ("ListLedgerEntries", Read),
        ("ListCoinPackages", Read),
        ("StartPurchase", Session),
        ("ConfirmPurchase", Session),
        ("CancelPurchase", Session),
        ("GetPurchase", Read),
        ("ListPurchases", Read),
        ("GetDailyBonus", Read),
        ("ClaimDailyBonus", Write),
    ]),
    ("tokuly.wallet.v1.EarningsService", &[
        ("GetEarningsSummary", Read),
        ("RequestPayout", Session),
        ("CancelPayoutRequest", Session),
        ("ListPayoutRequests", Read),
    ]),
    ("tokuly.wallet.v1.WalletAdminService", &[
        ("ListPayoutQueue", Session),
        ("GetPayoutRequest", Session),
        ("ApprovePayout", Session),
        ("RejectPayout", Session),
        ("MarkPayoutPaid", Session),
        ("GetUserWallet", Session),
        ("ListUserLedgerEntries", Session),
        ("AdjustBalance", Session),
        ("ListAdjustments", Session),
        ("ListAllCoinPackages", Session),
        ("SaveCoinPackage", Session),
    ]),
    ("tokuly.community.v1.CommunityService", &[
        ("CreateCommunity", Write),
        ("GetCommunity", Read),
        ("UpdateCommunity", Write),
        ("DeleteCommunity", Write),
        ("CheckHandle", Read),
        ("TransferOwnership", Write),
        ("ListMyCommunities", Read),
        ("ListUserCommunities", Read),
        ("JoinCommunity", Write),
        ("PreviewInvite", Read),
        ("LeaveCommunity", Write),
        ("CancelJoinRequest", Write),
        ("ListJoinRequests", Read),
        ("RespondJoinRequest", Write),
        ("ListMembers", Read),
        ("GetMember", Read),
        ("SetMemberRole", Write),
        ("UpdateMembership", Write),
        ("CreateInvite", Write),
        ("ListInvites", Read),
        ("RevokeInvite", Write),
    ]),
    ("tokuly.community.v1.CommunityBoardService", &[
        ("CreateChannel", Write),
        ("UpdateChannel", Write),
        ("DeleteChannel", Write),
        ("ReorderChannels", Write),
        ("ListChannels", Read),
        ("CreateThread", Write),
        ("GetThread", Read),
        ("UpdateThread", Write),
        ("DeleteThread", Write),
        ("ListThreads", Read),
        ("ListAnnouncements", Read),
        ("ListFollowedThreads", Read),
        ("PinThread", Write),
        ("LockThread", Write),
        ("SetAnnouncement", Write),
        ("SetThreadFollow", Write),
        ("MarkRead", Write),
        ("VotePoll", Write),
        ("ClosePoll", Write),
        ("CreateReply", Write),
        ("GetReply", Read),
        ("UpdateReply", Write),
        ("DeleteReply", Write),
        ("ListReplies", Read),
    ]),
    ("tokuly.community.v1.CommunityModerationService", &[
        ("RemoveMember", Write),
        ("BanMember", Write),
        ("UnbanMember", Write),
        ("ListBans", Read),
        ("TimeoutMember", Write),
        ("ClearTimeout", Write),
        ("GetSafetySettings", Read),
        ("UpdateSafetySettings", Write),
        ("SetSlowMode", Write),
        ("ListReviewQueue", Read),
        ("ReviewPost", Write),
        ("RevealAuthor", Session),
        ("GetContentHistory", Read),
        ("ListModerationLog", Read),
    ]),
    ("tokuly.community.v1.CommunityEventService", &[
        ("CreateEvent", Write),
        ("UpdateEvent", Write),
        ("CancelEvent", Write),
        ("DeleteEvent", Write),
        ("GetEvent", Read),
        ("ListEvents", Read),
        ("ListMyEvents", Read),
        ("Rsvp", Write),
        ("ListAttendees", Read),
        ("GetCheckInCode", Write),
        ("CheckIn", Write),
        ("CheckInAttendee", Write),
    ]),
    ("tokuly.community.v1.CommunityDiscoveryService", &[
        ("ListDirectory", Read),
        ("ListTrending", Read),
        ("ListFriendsCommunities", Read),
        ("ListSchoolCommunities", Read),
        ("ListPopularTags", Read),
    ]),
    ("tokuly.plans.v1.FreeService", &[
        ("SetFreeStatus", Write),
        ("ClearFreeStatus", Write),
        ("GetFreeStatus", Read),
        ("ListFreeFriends", Read),
        ("AnswerFreeStatus", Write),
        ("ListFreeAnswers", Read),
        ("SetFreeAlert", Write),
        ("ListFreeAlerts", Read),
    ]),
    ("tokuly.plans.v1.PlanService", &[
        ("CreatePlan", Write),
        ("GetPlan", Read),
        ("UpdatePlan", Write),
        ("CancelPlan", Write),
        ("ListPlans", Read),
        ("InviteToPlan", Write),
        ("RemovePlanParticipant", Write),
        ("LeavePlan", Write),
        ("SetPlanRole", Write),
        ("RespondToPlan", Write),
        ("AddChecklistItem", Write),
        ("UpdateChecklistItem", Write),
        ("RemoveChecklistItem", Write),
        ("AddExpense", Write),
        ("UpdateExpense", Write),
        ("RemoveExpense", Write),
        ("ListExpenses", Read),
        ("GetSplit", Read),
        ("SettleTransfer", Write),
        ("RemoveSettlement", Write),
    ]),
    ("tokuly.plans.v1.PollService", &[
        ("CreatePoll", Write),
        ("GetPoll", Read),
        ("UpdatePoll", Write),
        ("ListPolls", Read),
        ("InviteToPoll", Write),
        ("AnswerPoll", Write),
        ("ClosePoll", Write),
        ("ReopenPoll", Write),
        ("FinalizePoll", Write),
        ("DeletePoll", Write),
        ("LeavePoll", Write),
        ("RemovePollParticipant", Write),
        ("RemindPollParticipants", Write),
    ]),
    ("tokuly.plans.v1.AnniversaryService", &[
        ("CreateCountdown", Write),
        ("UpdateCountdown", Write),
        ("DeleteCountdown", Write),
        ("GetCountdown", Read),
        ("ListCountdowns", Read),
        ("StartFriendshipCounter", Write),
        ("InviteCouple", Write),
        ("RespondCoupleInvite", Write),
        ("UpdateAnniversary", Write),
        ("EndAnniversary", Write),
        ("ListAnniversaries", Read),
        ("ListBirthdays", Read),
        ("GetPlansSettings", Read),
        ("UpdatePlansSettings", Write),
    ]),
    ("tokuly.plans.v1.CalendarService", &[
        ("GetCalendar", Read),
    ]),
    ("tokuly.quest.v1.QuestService", &[
        // Also hands over level-up coins that were earned but not paid yet;
        // nothing is decided here, so it stays a read.
        ("GetProgress", Read),
        ("ListXpHistory", Read),
        ("ListEarningRules", Read),
        ("GetProfile", Read),
        ("GetPreferences", Read),
        ("UpdatePreferences", Write),
    ]),
    ("tokuly.quest.v1.MissionService", &[
        // Draws today's missions and pays claims already made, if needed.
        ("ListMissions", Read),
        // Puts the reward's coins into the wallet.
        ("ClaimMission", With(&["quest:write", "wallet:write"])),
        ("RerollMission", Write),
    ]),
    ("tokuly.quest.v1.BadgeService", &[
        ("ListBadgeCatalog", Read),
        ("GetBadge", Read),
        ("ListUserBadges", Read),
        ("SetShowcase", Write),
        ("SetTitle", Write),
    ]),
    ("tokuly.quest.v1.StreakService", &[
        ("ListStreaks", Read),
        ("GetStreak", Read),
        // Costs coins.
        ("RestoreStreak", With(&["quest:write", "wallet:write"])),
    ]),
    ("tokuly.quest.v1.LeaderboardService", &[
        ("ListLeaderboards", Read),
        ("GetLeaderboard", Read),
        ("GetSeason", Read),
        ("ListSeasonResults", Read),
    ]),
    ("tokuly.quest.v1.QuestAdminService", &[
        ("AwardBadge", Session),
        ("RevokeBadge", Session),
        ("ListBadgeActions", Session),
    ]),
    // Whatever returns audio credentials takes a session: they put a
    // listener (or a microphone) into a room of people, and that is a person
    // at a device, not a script. `spaces:read` shows who is in which room
    // right now and is offered as a sensitive scope.
    ("tokuly.spaces.v1.SpaceService", &[
        ("GetSpacesConfig", Read),
        // Goes live at once with the caller inside, unless scheduled.
        ("CreateSpace", Session),
        ("UpdateSpace", Write),
        ("StartSpace", Session),
        ("CancelSpace", Write),
        ("EndSpace", Write),
        ("DeleteSpace", Write),
        ("GetSpace", Read),
        ("JoinSpace", Session),
        ("LeaveSpace", Write),
        ("ListParticipants", Read),
        ("SetHandRaised", Write),
        ("InviteToStage", Write),
        // Makes the caller a speaker and returns credentials for it.
        ("RespondToStageInvite", Session),
        ("MoveToAudience", Write),
        ("SetCohost", Write),
        ("TransferHost", Write),
        ("MuteSpeaker", Write),
        ("RemoveParticipant", Write),
        ("UnbanUser", Write),
        ("ListBans", Read),
        ("SendReaction", Write),
        ("SetPin", Write),
        ("InviteToSpace", Write),
        ("RevokeSpaceInvite", Write),
        ("ListSpaceInvites", Read),
        ("SetReminder", Write),
        ("ListLiveSpaces", Read),
        ("ListUpcomingSpaces", Read),
        ("ListMySpaces", Read),
        ("ListPopularTags", Read),
        // Which friends are in a room right now, across the friend list.
        ("ListFriendsInRooms", With(&["spaces:read", "social:read"])),
    ]),
    ("tokuly.spaces.v1.HangoutService", &[
        ("CreateHangout", Write),
        ("UpdateHangout", Write),
        ("DeleteHangout", Write),
        ("GetHangout", Read),
        ("ListHangouts", Read),
        ("InviteToHangout", Write),
        ("RespondToHangoutInvite", Write),
        ("RemoveHangoutMember", Write),
        ("QuitHangout", Write),
        ("TransferHangout", Write),
        ("SetHangoutMuted", Write),
        ("JoinHangout", Session),
        ("LeaveHangout", Write),
    ]),
    ("tokuly.shorts.v1.ShortService", &[
        ("CreateShort", Write),
        ("UpdateShort", Write),
        ("DeleteShort", Write),
        ("GetShort", Read),
        ("BatchGetShorts", Read),
        ("ListMyShorts", Read),
        ("PinShort", Write),
        ("UnpinShort", Write),
        ("SaveShort", Write),
        ("UnsaveShort", Write),
        ("ListSavedShorts", Read),
        ("ListLikedShorts", Read),
        // Counts a share.
        ("ShareShort", Write),
        ("ListRemixes", Read),
    ]),
    ("tokuly.shorts.v1.ShortFeedService", &[
        ("GetForYouFeed", Read),
        ("GetFollowingFeed", Read),
        ("GetFriendsFeed", Read),
        // Counts views and marks shorts as seen.
        ("ReportPlayback", Write),
        ("MarkNotInterested", Write),
        ("UndoNotInterested", Write),
    ]),
    ("tokuly.shorts.v1.ShortDiscoveryService", &[
        ("ListChannelShorts", Read),
        ("ListUserShorts", Read),
        ("GetHashtag", Read),
        ("ListHashtagShorts", Read),
        ("ListTrendingHashtags", Read),
        ("SuggestHashtags", Read),
        ("GetSound", Read),
        ("ListSoundShorts", Read),
        ("SuggestSounds", Read),
    ]),
    // The handlers answer only for the caller's own shorts and channels.
    ("tokuly.shorts.v1.ShortAnalyticsService", &[
        ("GetShortAnalytics", Read),
        ("GetChannelShortsAnalytics", Read),
    ]),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn scopes(path: &str) -> Vec<String> {
        match access_for(path) {
            Some(Access::Scopes(scopes)) => scopes,
            other => panic!("{path} takes scopes, got {other:?}"),
        }
    }

    #[test]
    fn listed_rpcs_need_the_scope_written_next_to_them() {
        assert_eq!(scopes("/tokuly.geo.v1.LocationService/ListFriendLocations"), ["geo:read"]);
        assert_eq!(scopes("tokuly.geo.v1.LocationService/ReportLocations"), ["geo:write"]);
        assert_eq!(scopes("/tokuly.notes.v1.NoteService/CreateNote"), ["notes:write"]);
        assert_eq!(scopes("/tokuly.identity.v1.AccountService/GetMe"), ["identity:read"]);
        assert_eq!(scopes("/tokuly.search.v1.SearchService/Suggest"), ["search:read"]);
        // Actions whose names sound like reading.
        assert_eq!(scopes("/tokuly.geo.v1.PlaceService/CheckIn"), ["geo:write"]);
        assert_eq!(scopes("/tokuly.media.v1.MediaService/GetUploadParts"), ["media:write"]);
        assert_eq!(scopes("/tokuly.chat.v1.ConversationService/MarkRead"), ["chat:write"]);
    }

    #[test]
    fn quest_spaces_and_shorts_follow_the_same_rules() {
        // Coins move only with the wallet scope.
        assert_eq!(scopes("/tokuly.quest.v1.MissionService/ClaimMission"), ["quest:write", "wallet:write"]);
        assert_eq!(scopes("/tokuly.quest.v1.StreakService/RestoreStreak"), ["quest:write", "wallet:write"]);
        assert_eq!(scopes("/tokuly.quest.v1.MissionService/RerollMission"), ["quest:write"]);
        // Staff.
        for method in ["AwardBadge", "RevokeBadge", "ListBadgeActions"] {
            let path = format!("/tokuly.quest.v1.QuestAdminService/{method}");
            assert_eq!(access_for(&path), Some(Access::SessionOnly), "{method}");
        }
        // Audio credentials are not for tokens.
        for path in [
            "/tokuly.spaces.v1.SpaceService/CreateSpace",
            "/tokuly.spaces.v1.SpaceService/StartSpace",
            "/tokuly.spaces.v1.SpaceService/JoinSpace",
            "/tokuly.spaces.v1.SpaceService/RespondToStageInvite",
            "/tokuly.spaces.v1.HangoutService/JoinHangout",
        ] {
            assert_eq!(access_for(path), Some(Access::SessionOnly), "{path}");
        }
        assert_eq!(scopes("/tokuly.spaces.v1.SpaceService/ListParticipants"), ["spaces:read"]);
        assert_eq!(scopes("/tokuly.spaces.v1.SpaceService/ListFriendsInRooms"), ["spaces:read", "social:read"]);
        assert_eq!(scopes("/tokuly.spaces.v1.SpaceService/RemoveParticipant"), ["spaces:write"]);
        assert_eq!(scopes("/tokuly.shorts.v1.ShortFeedService/ReportPlayback"), ["shorts:write"]);
        assert_eq!(scopes("/tokuly.shorts.v1.ShortDiscoveryService/SuggestSounds"), ["shorts:read"]);
        assert_eq!(scopes("/tokuly.shorts.v1.ShortAnalyticsService/GetShortAnalytics"), ["shorts:read"]);
    }

    #[test]
    fn live_streams_keep_their_keys_tokens_and_the_air_to_sessions() {
        // The stream key, going on air, sending the audience away, widget tokens, appointing moderators.
        for path in [
            "/tokuly.live.v1.LiveStreamService/GetIngestInfo",
            "/tokuly.live.v1.LiveStreamService/RotateStreamKey",
            "/tokuly.live.v1.LiveStreamService/StartStream",
            "/tokuly.live.v1.LiveStreamService/RaidStream",
            "/tokuly.live.v1.OverlayService/ListOverlays",
            "/tokuly.live.v1.OverlayService/CreateOverlay",
            "/tokuly.live.v1.OverlayService/UpdateOverlay",
            "/tokuly.live.v1.OverlayService/DeleteOverlay",
            "/tokuly.live.v1.OverlayService/RotateOverlayToken",
            "/tokuly.live.v1.LiveChatService/AddModerator",
            "/tokuly.live.v1.LiveChatService/RemoveModerator",
        ] {
            assert_eq!(access_for(path), Some(Access::SessionOnly), "{path}");
        }
        // Coins move only with the wallet scope, and what was paid is read with it.
        assert_eq!(scopes("/tokuly.live.v1.LiveGiftService/SendGift"), ["live:write", "wallet:write"]);
        assert_eq!(scopes("/tokuly.live.v1.LiveGiftService/ListSentGifts"), ["live:read", "wallet:read"]);
        assert_eq!(scopes("/tokuly.live.v1.LiveGiftService/ListReceivedGifts"), ["live:read", "wallet:read"]);
        assert_eq!(scopes("/tokuly.live.v1.LiveStreamService/ListFriendStreams"), ["live:read", "social:read"]);
        // The rest reads or writes the domain.
        assert_eq!(scopes("/tokuly.live.v1.LiveStreamService/GetStream"), ["live:read"]);
        assert_eq!(scopes("/tokuly.live.v1.LiveStreamService/ListViewers"), ["live:read"]);
        assert_eq!(scopes("/tokuly.live.v1.LiveStreamService/EndStream"), ["live:write"]);
        assert_eq!(scopes("/tokuly.live.v1.LiveChatService/RestrictChatUser"), ["live:write"]);
        assert_eq!(scopes("/tokuly.live.v1.OverlayService/GetOverlay"), ["live:read"]);
        let live = classified().filter(|(path, _)| path.starts_with("/tokuly.live.v1.")).count();
        assert_eq!(live, 57);
    }

    #[test]
    fn what_carries_location_or_messages_needs_those_scopes_too() {
        // Friends' coordinates, wherever the RPC lives.
        assert_eq!(scopes("/tokuly.feed.v1.MapStoryService/ListMapStories"), ["feed:read", "geo:read"]);
        assert_eq!(scopes("/tokuly.feed.v1.ActivityService/ListFriendsActivity"), ["feed:read", "geo:read"]);
        assert_eq!(scopes("/tokuly.identity.v1.ProfileService/GetProfile"), ["identity:read", "geo:read"]);
        assert_eq!(scopes("/tokuly.social.v1.FriendService/DiscoverNearby"), ["social:write", "geo:read"]);
        // The inbox holds message previews and place alerts.
        assert_eq!(
            scopes("/tokuly.notify.v1.InboxService/ListNotifications"),
            ["notify:read", "chat:read", "geo:read"]
        );
        // A report about a chat message quotes the message.
        assert_eq!(scopes("/tokuly.engagement.v1.ReportService/ListMyReports"), ["engagement:read", "chat:read"]);
        // News is neither.
        assert_eq!(scopes("/tokuly.feed.v1.NewsService/ListNews"), ["feed:read"]);
    }

    #[test]
    fn credentials_money_out_emergencies_and_moderation_take_a_session() {
        for path in [
            "/tokuly.identity.v1.AuthService/SignIn",
            "/tokuly.identity.v1.AuthService/CheckHandle",
            "/tokuly.identity.v1.AccountService/ChangePassword",
            "/tokuly.identity.v1.AccountService/RequestAccountDeletion",
            "/tokuly.identity.v1.AccountService/RequestDataExport",
            "/tokuly.identity.v1.AccountService/BeginConnection",
            "/tokuly.identity.v1.AccountService/FinishConnection",
            "/tokuly.identity.v1.AccountService/DeleteConnection",
            "/tokuly.geo.v1.SafetyService/TriggerSos",
            "/tokuly.geo.v1.SafetyService/ResolveSos",
            "/tokuly.wallet.v1.EarningsService/RequestPayout",
            "/tokuly.wallet.v1.WalletService/StartPurchase",
            "/tokuly.admin.v1.UserAdminService/SuspendUser",
            "/tokuly.admin.v1.AnnouncementService/ListAnnouncements",
            "/tokuly.wallet.v1.WalletAdminService/AdjustBalance",
            "/tokuly.search.v1.SearchAdminService/Reindex",
            "/tokuly.dev.v1.TokenService/CreateToken",
            "/tokuly.dev.v1.AppService/CreateApp",
            "/tokuly.dev.v1.WebhookService/CreateWebhook",
            "/tokuly.dev.v1.WebhookService/RotateWebhookSecret",
            "/tokuly.dev.v1.WebhookService/ReplayDelivery",
            "/tokuly.identity.v1.AccountService/ListConnections",
            "/tokuly.community.v1.CommunityModerationService/RevealAuthor",
        ] {
            assert_eq!(access_for(path), Some(Access::SessionOnly), "{path}");
        }
        assert_eq!(access_for("/tokuly.dev.v1.TokenService/GetCurrentToken"), Some(Access::AnyToken));
        assert_eq!(access_for("/tokuly.dev.v1.ReferenceService/ListServices"), Some(Access::AnyToken));
        assert_eq!(scopes("/tokuly.dev.v1.WebhookService/GetDelivery"), ["dev:read"]);
    }

    #[test]
    fn spending_coins_takes_the_wallet_scope_wherever_it_happens() {
        assert_eq!(scopes("/tokuly.stamp.v1.StampLibraryService/PurchasePack"), ["stamp:write", "wallet:write"]);
        assert_eq!(scopes("/tokuly.ask.v1.ComplimentService/BuyHint"), ["ask:write", "wallet:write"]);
        assert_eq!(scopes("/tokuly.wallet.v1.WalletService/ClaimDailyBonus"), ["wallet:write"]);
        // A round's own reward is not spending.
        assert_eq!(scopes("/tokuly.ask.v1.ComplimentService/Vote"), ["ask:write"]);
        // Splitting costs among friends moves no coins.
        assert_eq!(scopes("/tokuly.plans.v1.PlanService/SettleTransfer"), ["plans:write"]);
    }

    #[test]
    fn nothing_is_inferred_for_an_rpc_that_is_not_listed() {
        for path in [
            // Names that a prefix rule would have taken for reads.
            "/tokuly.notes.v1.NoteService/BatchDeleteNotes",
            "/tokuly.video.v1.PlaylistService/WatchLater",
            "/tokuly.wallet.v1.WalletService/CheckoutCart",
            // Services and versions that do not exist.
            "/tokuly.live.v1.LiveService/GetStreamKey",
            "/tokuly.notes.v1.NewService/GetThing",
            "/tokuly.notes.v2.NoteService/GetNote",
            // Not RPC paths at all.
            "/",
            "",
            "/healthz",
            "/oauth/token",
            "/tokuly.notes.v1.NoteService",
            "/tokuly.notes.v1.NoteService/",
            "/tokuly.notes.v1.NoteService/GetNote/more",
            "//tokuly.notes.v1.NoteService/GetNote",
            "/tokuly.notes.v1.NoteService/getnote",
        ] {
            assert_eq!(access_for(path), None, "{path:?}");
        }
    }

    #[test]
    fn the_table_lists_each_rpc_once_with_well_formed_scopes() {
        let mut paths: Vec<String> = classified().map(|(path, _)| path).collect();
        let listed = paths.len();
        assert_eq!(listed, TABLE.iter().map(|(_, methods)| methods.len()).sum::<usize>(), "every row resolves");
        paths.sort();
        paths.dedup();
        assert_eq!(paths.len(), listed, "no RPC is listed twice");
        for (path, access) in classified() {
            if let Access::Scopes(scopes) = access {
                assert!(
                    !scopes.is_empty() && scopes.iter().all(|scope| crate::api_tokens::is_scope(scope)),
                    "{path}: {scopes:?}"
                );
            }
        }
    }
}
