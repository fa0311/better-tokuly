//! The API reference as served: the RPCs and the raw descriptor set.
use buffa::Message as _;
use tokuly_proto::dev::*;

use crate::common::*;

const REFERENCE_SERVICE: &str = "tokuly.dev.v1.ReferenceService";

#[tokio::test]
async fn the_server_describes_its_own_api() {
    let ctx = start().await;
    let alice = ctx.app.user("alice").await;
    // No sign-in needed; a token of any scope may read it too.
    let all: ListServicesResponse = ctx
        .app
        .call(&format!("{REFERENCE_SERVICE}/ListServices"), None, &ListServicesRequest::default())
        .await
        .unwrap();
    assert!(
        all.services.iter().any(|s| s.name == "tokuly.identity.v1.AccountService")
            && all.services.iter().any(|s| s.name == "tokuly.geo.v1.LocationService")
    );
    let (_, secret) = personal_token(&ctx, &alice, &["notes:read"]).await;
    let dev: ListServicesResponse = ctx
        .app
        .call(
            &format!("{REFERENCE_SERVICE}/ListServices"),
            Some(&with_token(&alice, &secret)),
            &ListServicesRequest { domain: "dev".into(), ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!(dev.services.len(), 7);
    assert!(dev.services.iter().all(|s| s.domain == "dev" && !s.description.is_empty()));
    let methods: usize = dev.services.iter().map(|s| s.methods.len()).sum();
    assert_eq!(methods, 48, "every RPC of this domain is listed");

    // What the reference says a token needs is what the middleware enforces.
    let identity: ListServicesResponse = ctx
        .app
        .call(
            &format!("{REFERENCE_SERVICE}/ListServices"),
            None,
            &ListServicesRequest { domain: "identity".into(), ..Default::default() },
        )
        .await
        .unwrap();
    let account = identity.services.iter().find(|s| s.name.ends_with("AccountService")).unwrap();
    let access = |name: &str| {
        account
            .methods
            .iter()
            .find(|m| m.name == name)
            .map(|m| (m.access.as_known().unwrap(), m.scope.clone()))
            .unwrap()
    };
    assert_eq!(access("GetMe"), (ApiAccess::API_ACCESS_SCOPE, "identity:read".to_owned()));
    assert_eq!(access("UpdateProfile"), (ApiAccess::API_ACCESS_SCOPE, "identity:write".to_owned()));
    assert_eq!(access("ChangePassword"), (ApiAccess::API_ACCESS_SESSION_ONLY, String::new()));

    let got: GetMethodResponse = ctx
        .app
        .call(
            &format!("{REFERENCE_SERVICE}/GetMethod"),
            None,
            &GetMethodRequest { path: "/tokuly.dev.v1.WebhookService/CreateWebhook".into(), ..Default::default() },
        )
        .await
        .unwrap();
    let method = got.method.as_option().unwrap();
    assert!(method.description.contains("WEBHOOK_URL_INVALID"));
    let schema: serde_json::Value = serde_json::from_str(&method.request_schema).unwrap();
    assert_eq!(schema["$defs"]["tokuly.dev.v1.CreateWebhookRequest"]["properties"]["eventTypes"]["type"], "array");
    assert!(serde_json::from_str::<serde_json::Value>(&method.response_schema).is_ok());
    let missing = ctx
        .app
        .call::<_, GetMethodResponse>(
            &format!("{REFERENCE_SERVICE}/GetMethod"),
            None,
            &GetMethodRequest { path: "tokuly.dev.v1.WebhookService/Explode".into(), ..Default::default() },
        )
        .await;
    assert_eq!(
        (missing.as_ref().unwrap_err().code.as_str(), missing.as_ref().unwrap_err().reason.as_str()),
        ("not_found", "METHOD_NOT_FOUND")
    );
}

#[tokio::test]
async fn the_raw_descriptor_set_is_served_for_code_generators() {
    let ctx = start().await;
    let reply = ctx.get("/reference/descriptor.binpb").await;
    assert_eq!((reply.status, reply.header("content-type")), (200, "application/x-protobuf"));
    let set = buffa_descriptor::generated::descriptor::FileDescriptorSet::decode_from_slice(&reply.body)
        .expect("a FileDescriptorSet");
    let dev = set
        .file
        .iter()
        .find(|file| file.name.as_deref() == Some("tokuly/dev/v1/dev.proto"))
        .expect("this domain's file");
    assert_eq!(dev.service.len(), 7);
    assert!(dev.source_code_info.as_option().is_some_and(|info| !info.location.is_empty()), "comments are included");
}
