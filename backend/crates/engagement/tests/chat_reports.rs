//! Reports about chat messages, through the real chat domain: what a
//! sender edited or unsent stays reportable, the moderators get every
//! version, and the reporter's own copy shows only what they may see now.
use tokuly_chat::{Chat, ChatConfig};
use tokuly_engagement::{Engagement, EngagementConfig};
use tokuly_proto::{
    chat::{
        DeleteMessageRequest, DeleteMessageResponse, EditMessageRequest, EditMessageResponse, OpenDirectRequest,
        OpenDirectResponse, SendMessageRequest, SendMessageResponse,
    },
    common::{SubjectRef, SubjectType},
    engagement::{
        CreateReportRequest, CreateReportResponse, ListMyReportsRequest, ListMyReportsResponse, ReportCategory,
    },
    social::{SendFriendRequestRequest, send_friend_request_request::Target},
};
use tokuly_social::{Social, SocialConfig};
use tokuly_stamp::{Stamp, StampConfig};
use tokuly_testkit::{TestApp, TestUser};
use uuid::Uuid;

const MIGRATIONS: &[&str] = &["0001", "0100", "0200", "0400", "0500", "0600", "1100", "1400"];

async fn start() -> TestApp {
    TestApp::start(MIGRATIONS, |core, identity, router| {
        let social = Social::new(core.clone(), identity.clone(), SocialConfig::default()).expect("social domain");
        let stamp =
            Stamp::new(core.clone(), identity.clone(), StampConfig { seed_official: false, ..Default::default() })
                .expect("stamp domain");
        let chat = Chat::new(core.clone(), identity.clone(), ChatConfig::default()).expect("chat domain");
        let engagement = Engagement::new(
            core.clone(),
            identity.clone(),
            EngagementConfig { reports_per_hour: 1000, ..Default::default() },
        )
        .expect("engagement domain");
        engagement.register(chat.register(stamp.register(social.register(router))))
    })
    .await
}

async fn befriend(app: &TestApp, a: &TestUser, b: &TestUser) {
    for (from, to) in [(a, b), (b, a)] {
        let request =
            SendFriendRequestRequest { target: Some(Target::UserId(to.id.to_string())), ..Default::default() };
        let _: serde_json::Value =
            app.call("tokuly.social.v1.FriendService/SendFriendRequest", Some(from), &request).await.unwrap();
    }
}

async fn send(app: &TestApp, sender: &TestUser, conversation_id: &str, text: &str) -> String {
    let request = SendMessageRequest {
        conversation_id: conversation_id.to_owned(),
        client_id: Uuid::now_v7().to_string(),
        text: text.to_owned(),
        ..Default::default()
    };
    let sent: SendMessageResponse =
        app.call("tokuly.chat.v1.MessageService/SendMessage", Some(sender), &request).await.unwrap();
    sent.message.id.clone()
}

async fn report(app: &TestApp, reporter: &TestUser, message_id: &str) -> CreateReportResponse {
    let request = CreateReportRequest {
        subject: SubjectRef {
            r#type: SubjectType::SUBJECT_TYPE_CHAT_MESSAGE.into(),
            id: message_id.to_owned(),
            ..Default::default()
        }
        .into(),
        category: ReportCategory::REPORT_CATEGORY_HARASSMENT.into(),
        ..Default::default()
    };
    app.call("tokuly.engagement.v1.ReportService/CreateReport", Some(reporter), &request).await.unwrap()
}

/// The `engagement.report.created` payload of a report.
async fn sent_to_moderators(app: &TestApp, report_id: &str) -> serde_json::Value {
    app.events("engagement.report.created")
        .await
        .into_iter()
        .find(|event| event.data["report_id"] == report_id)
        .expect("the report reached the moderators")
        .data
}

#[tokio::test]
async fn a_reported_message_shows_its_old_versions_to_the_moderators_only() {
    let app = start().await;
    let (taro, hanako) = (app.user("taro").await, app.user("hanako").await);
    befriend(&app, &taro, &hanako).await;
    let opened: OpenDirectResponse = app
        .call(
            "tokuly.chat.v1.ConversationService/OpenDirect",
            Some(&taro),
            &OpenDirectRequest { user_id: hanako.id.to_string(), ..Default::default() },
        )
        .await
        .unwrap();
    let conversation_id = opened.conversation.id.clone();

    // Edited: the reporter sees what it says now, the moderators every version.
    let edited = send(&app, &taro, &conversation_id, "おはよう").await;
    for text in ["ばか！", "なんでもない"] {
        let edit = EditMessageRequest { message_id: edited.clone(), text: text.to_owned(), ..Default::default() };
        let _: EditMessageResponse =
            app.call("tokuly.chat.v1.MessageService/EditMessage", Some(&taro), &edit).await.unwrap();
    }
    let filed = report(&app, &hanako, &edited).await;
    assert_eq!(filed.report.subject_title, "なんでもない");
    let event = sent_to_moderators(&app, &filed.report.id).await;
    assert_eq!(event["subject_title"], "なんでもない");
    assert_eq!(event["reported_content"], "[3版] おはよう → ばか！ → なんでもない");

    // Unsent: the reporter's copy names a message, nothing it said.
    let unsent = send(&app, &taro, &conversation_id, "おまえなんか大嫌いだ").await;
    let delete = DeleteMessageRequest { message_id: unsent.clone(), ..Default::default() };
    let _: DeleteMessageResponse =
        app.call("tokuly.chat.v1.MessageService/DeleteMessage", Some(&taro), &delete).await.unwrap();
    let filed = report(&app, &hanako, &unsent).await;
    assert_eq!(filed.report.subject_title, "メッセージ");
    let event = sent_to_moderators(&app, &filed.report.id).await;
    assert_eq!(event["subject_title"], "メッセージ");
    assert_eq!(event["reported_content"], "おまえなんか大嫌いだ");

    // The reporter's list keeps no text the sender took back.
    let mine: ListMyReportsResponse = app
        .call("tokuly.engagement.v1.ReportService/ListMyReports", Some(&hanako), &ListMyReportsRequest::default())
        .await
        .unwrap();
    let titles: Vec<&str> = mine.reports.iter().map(|report| report.subject_title.as_str()).collect();
    assert_eq!(titles, ["メッセージ", "なんでもない"]);
}
