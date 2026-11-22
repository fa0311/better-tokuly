//! The API reference, read from the descriptor set the server embeds
//! (`tokuly_proto::FILE_DESCRIPTOR_SET`): services and methods with the
//! comments of the proto files, what a token needs to call each, and JSON
//! Schemas of requests and responses in their JSON form.
use std::collections::{BTreeMap, HashMap};

use buffa::Message as _;
use buffa_descriptor::generated::descriptor::{
    DescriptorProto, EnumDescriptorProto, FieldDescriptorProto, FileDescriptorProto, FileDescriptorSet,
    field_descriptor_proto::{Label, Type},
};
use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use serde_json::{Value, json};
use tokuly_core::{
    api_tokens::{Access, access_for},
    prelude::*,
};
use tokuly_proto::dev::*;

use crate::Dev;

struct MethodDoc {
    name: String,
    description: String,
    request_type: String,
    response_type: String,
    client_streaming: bool,
    server_streaming: bool,
}

struct ServiceDoc {
    /// `tokuly.notes.v1.NoteService`.
    name: String,
    /// `notes`.
    domain: String,
    description: String,
    methods: Vec<MethodDoc>,
}

enum FieldType {
    /// A scalar, as its JSON Schema.
    Scalar(Value),
    Message(String),
    Enum(String),
}

struct FieldDoc {
    json_name: String,
    description: String,
    field_type: FieldType,
    repeated: bool,
    /// The oneof the field belongs to.
    oneof: Option<String>,
}

struct MessageDoc {
    description: String,
    fields: Vec<FieldDoc>,
    /// A synthetic `map<K, V>` entry.
    map_entry: bool,
}

struct EnumDoc {
    description: String,
    /// Value names with their comments.
    values: Vec<(String, String)>,
}

/// The API as its descriptor set describes it.
pub struct Reference {
    services: Vec<ServiceDoc>,
    messages: HashMap<String, MessageDoc>,
    enums: HashMap<String, EnumDoc>,
}

/// The comments of one file, by the path of what they are attached to
/// (`descriptor.proto`, `SourceCodeInfo.Location.path`).
struct Comments(HashMap<Vec<i32>, String>);

impl Comments {
    fn of(file: &FileDescriptorProto) -> Self {
        let mut comments = HashMap::new();
        if let Some(info) = file.source_code_info.as_option() {
            for location in &info.location {
                let text = location
                    .leading_comments
                    .as_deref()
                    .or(location.trailing_comments.as_deref())
                    .map(tidy)
                    .unwrap_or_default();
                if !text.is_empty() {
                    comments.insert(location.path.clone(), text);
                }
            }
        }
        Self(comments)
    }

    fn at(&self, path: &[i32]) -> String {
        self.0.get(path).cloned().unwrap_or_default()
    }
}

/// A comment as prose: the space after each `//` and trailing blanks removed.
fn tidy(comment: &str) -> String {
    comment
        .lines()
        .map(|line| line.strip_prefix(' ').unwrap_or(line).trim_end())
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_owned()
}

/// Appends `tag, index` to a location path.
fn child(path: &[i32], tag: i32, index: usize) -> Vec<i32> {
    let mut path = path.to_vec();
    path.extend([tag, index as i32]);
    path
}

// Field numbers of descriptor.proto that location paths are made of.
const FILE_MESSAGE: i32 = 4;
const FILE_ENUM: i32 = 5;
const FILE_SERVICE: i32 = 6;
const SERVICE_METHOD: i32 = 2;
const MESSAGE_FIELD: i32 = 2;
const MESSAGE_NESTED: i32 = 3;
const MESSAGE_ENUM: i32 = 4;
const ENUM_VALUE: i32 = 2;

fn type_name(name: &Option<String>) -> String {
    name.as_deref().unwrap_or_default().trim_start_matches('.').to_owned()
}

/// A 64-bit integer: a string in canonical proto3 JSON, though parsers
/// accept a number too.
fn int64(format: &str) -> Value {
    json!({ "type": ["string", "integer"], "format": format })
}

fn scalar(field_type: Option<Type>) -> Value {
    match field_type {
        Some(Type::TYPE_DOUBLE | Type::TYPE_FLOAT) => json!({ "type": "number" }),
        Some(Type::TYPE_INT32 | Type::TYPE_SINT32 | Type::TYPE_SFIXED32) => {
            json!({ "type": "integer", "format": "int32" })
        }
        Some(Type::TYPE_UINT32 | Type::TYPE_FIXED32) => json!({ "type": "integer", "format": "uint32", "minimum": 0 }),
        Some(Type::TYPE_INT64 | Type::TYPE_SINT64 | Type::TYPE_SFIXED64) => int64("int64"),
        Some(Type::TYPE_UINT64 | Type::TYPE_FIXED64) => int64("uint64"),
        Some(Type::TYPE_BOOL) => json!({ "type": "boolean" }),
        Some(Type::TYPE_BYTES) => json!({ "type": "string", "contentEncoding": "base64" }),
        _ => json!({ "type": "string" }),
    }
}

/// Messages with a JSON form of their own (proto3 JSON mapping).
fn well_known(name: &str) -> Option<Value> {
    Some(match name {
        "google.protobuf.Timestamp" => json!({ "type": "string", "format": "date-time" }),
        "google.protobuf.Duration" => json!({ "type": "string", "pattern": "^-?[0-9]+(\\.[0-9]+)?s$" }),
        "google.protobuf.FieldMask" => json!({ "type": "string" }),
        "google.protobuf.Struct" => json!({ "type": "object" }),
        "google.protobuf.Value" => json!({}),
        "google.protobuf.ListValue" => json!({ "type": "array" }),
        "google.protobuf.Empty" => json!({ "type": "object", "additionalProperties": false }),
        "google.protobuf.StringValue" => json!({ "type": "string" }),
        "google.protobuf.BoolValue" => json!({ "type": "boolean" }),
        "google.protobuf.BytesValue" => json!({ "type": "string", "contentEncoding": "base64" }),
        "google.protobuf.DoubleValue" | "google.protobuf.FloatValue" => json!({ "type": "number" }),
        "google.protobuf.Int32Value" | "google.protobuf.UInt32Value" => json!({ "type": "integer" }),
        "google.protobuf.Int64Value" => int64("int64"),
        "google.protobuf.UInt64Value" => int64("uint64"),
        _ => return None,
    })
}

impl Reference {
    pub fn load(descriptor_set: &[u8]) -> anyhow::Result<Self> {
        let set = FileDescriptorSet::decode_from_slice(descriptor_set)
            .map_err(|e| anyhow::anyhow!("the embedded API descriptor set does not decode: {e}"))?;
        let mut reference = Self { services: Vec::new(), messages: HashMap::new(), enums: HashMap::new() };
        for file in &set.file {
            let package = file.package.clone().unwrap_or_default();
            let comments = Comments::of(file);
            for (i, message) in file.message_type.iter().enumerate() {
                reference.add_message(&package, message, &[FILE_MESSAGE, i as i32], &comments);
            }
            for (i, declared) in file.enum_type.iter().enumerate() {
                reference.add_enum(&package, declared, &[FILE_ENUM, i as i32], &comments);
            }
            // Only our own services are the API.
            let Some(domain) = package.strip_prefix("tokuly.").and_then(|rest| rest.split('.').next()) else {
                continue;
            };
            for (i, service) in file.service.iter().enumerate() {
                let path = [FILE_SERVICE, i as i32];
                let methods = service
                    .method
                    .iter()
                    .enumerate()
                    .map(|(j, method)| MethodDoc {
                        name: method.name.clone().unwrap_or_default(),
                        description: comments.at(&child(&path, SERVICE_METHOD, j)),
                        request_type: type_name(&method.input_type),
                        response_type: type_name(&method.output_type),
                        client_streaming: method.client_streaming.unwrap_or(false),
                        server_streaming: method.server_streaming.unwrap_or(false),
                    })
                    .collect();
                reference.services.push(ServiceDoc {
                    name: format!("{package}.{}", service.name.as_deref().unwrap_or_default()),
                    domain: domain.to_owned(),
                    description: comments.at(&path),
                    methods,
                });
            }
        }
        reference.services.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(reference)
    }

    fn add_enum(&mut self, scope: &str, declared: &EnumDescriptorProto, path: &[i32], comments: &Comments) {
        let values = declared
            .value
            .iter()
            .enumerate()
            .map(|(i, value)| (value.name.clone().unwrap_or_default(), comments.at(&child(path, ENUM_VALUE, i))))
            .collect();
        self.enums.insert(
            format!("{scope}.{}", declared.name.as_deref().unwrap_or_default()),
            EnumDoc { description: comments.at(path), values },
        );
    }

    fn add_message(&mut self, scope: &str, message: &DescriptorProto, path: &[i32], comments: &Comments) {
        let name = format!("{scope}.{}", message.name.as_deref().unwrap_or_default());
        for (i, nested) in message.nested_type.iter().enumerate() {
            self.add_message(&name, nested, &child(path, MESSAGE_NESTED, i), comments);
        }
        for (i, declared) in message.enum_type.iter().enumerate() {
            self.add_enum(&name, declared, &child(path, MESSAGE_ENUM, i), comments);
        }
        let field = |(i, field): (usize, &FieldDescriptorProto)| FieldDoc {
            json_name: field.json_name.clone().or_else(|| field.name.clone()).unwrap_or_default(),
            description: comments.at(&child(path, MESSAGE_FIELD, i)),
            field_type: match field.r#type {
                Some(Type::TYPE_MESSAGE | Type::TYPE_GROUP) => FieldType::Message(type_name(&field.type_name)),
                Some(Type::TYPE_ENUM) => FieldType::Enum(type_name(&field.type_name)),
                other => FieldType::Scalar(scalar(other)),
            },
            repeated: field.label == Some(Label::LABEL_REPEATED),
            // A proto3 `optional` field is a oneof only on the wire.
            oneof: field
                .oneof_index
                .filter(|_| !field.proto3_optional.unwrap_or(false))
                .and_then(|i| message.oneof_decl.get(i as usize))
                .and_then(|oneof| oneof.name.clone()),
        };
        let doc = MessageDoc {
            description: comments.at(path),
            fields: message.field.iter().enumerate().map(field).collect(),
            map_entry: message.options.as_option().and_then(|options| options.map_entry).unwrap_or(false),
        };
        self.messages.insert(name, doc);
    }

    /// The schema of one field's value, noting which definitions it needs.
    fn field_schema(&self, field: &FieldDoc, needed: &mut Vec<String>) -> Value {
        let reference = |name: &str, needed: &mut Vec<String>| match well_known(name) {
            Some(schema) => schema,
            None => {
                needed.push(name.to_owned());
                json!({ "$ref": format!("#/$defs/{name}") })
            }
        };
        let mut schema = match &field.field_type {
            FieldType::Scalar(schema) => schema.clone(),
            FieldType::Enum(name) => reference(name, needed),
            FieldType::Message(name) => match self
                .messages
                .get(name)
                .filter(|message| message.map_entry && field.repeated)
            {
                // map<K, V>: an object whose values are V.
                Some(entry) => {
                    let value = entry
                        .fields
                        .iter()
                        .find(|f| f.json_name == "value")
                        .map(|f| self.field_schema(f, needed))
                        .unwrap_or_else(|| json!({}));
                    return describe(json!({ "type": "object", "additionalProperties": value }), &field.description);
                }
                None => reference(name, needed),
            },
        };
        if field.repeated {
            schema = json!({ "type": "array", "items": schema });
        }
        describe(schema, &field.description)
    }

    fn definition(&self, name: &str, needed: &mut Vec<String>) -> Value {
        if let Some(declared) = self.enums.get(name) {
            let values: Vec<&str> = declared.values.iter().map(|(name, _)| name.as_str()).collect();
            let notes: BTreeMap<&str, &str> = declared
                .values
                .iter()
                .filter(|(_, note)| !note.is_empty())
                .map(|(name, note)| (name.as_str(), note.as_str()))
                .collect();
            let mut schema = json!({ "type": "string", "enum": values });
            if !notes.is_empty() {
                schema["x-enum-descriptions"] = json!(notes);
            }
            return describe(schema, &declared.description);
        }
        let Some(message) = self.messages.get(name) else { return json!({}) };
        let mut properties = serde_json::Map::new();
        // Which fields exclude each other, by oneof.
        let mut oneofs: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for field in &message.fields {
            properties.insert(field.json_name.clone(), self.field_schema(field, needed));
            if let Some(oneof) = &field.oneof {
                oneofs.entry(oneof).or_default().push(&field.json_name);
            }
        }
        let mut schema = json!({ "type": "object", "properties": properties });
        if !oneofs.is_empty() {
            schema["x-oneofs"] = json!(oneofs);
        }
        describe(schema, &message.description)
    }

    /// A self-contained JSON Schema (2020-12) for the JSON form of a message.
    pub fn schema(&self, message: &str) -> Value {
        let mut definitions = BTreeMap::new();
        let mut needed = vec![message.to_owned()];
        while let Some(name) = needed.pop() {
            if !definitions.contains_key(&name) {
                let definition = self.definition(&name, &mut needed);
                definitions.insert(name, definition);
            }
        }
        json!({ "$schema": "https://json-schema.org/draft/2020-12/schema", "$ref": format!("#/$defs/{message}"), "$defs": definitions })
    }

    fn method_proto(&self, service: &ServiceDoc, method: &MethodDoc, with_schemas: bool) -> ApiMethod {
        let path = format!("/{}/{}", service.name, method.name);
        let (access, scope) = match access_for(&path) {
            Some(Access::Scopes(scopes)) => (ApiAccess::API_ACCESS_SCOPE, scopes.join(" ")),
            Some(Access::AnyToken) => (ApiAccess::API_ACCESS_ANY_TOKEN, String::new()),
            Some(Access::SessionOnly) | None => (ApiAccess::API_ACCESS_SESSION_ONLY, String::new()),
        };
        let schema = |message: &str| if with_schemas { self.schema(message).to_string() } else { String::new() };
        ApiMethod {
            name: method.name.clone(),
            service: service.name.clone(),
            path,
            description: method.description.clone(),
            request_type: method.request_type.clone(),
            response_type: method.response_type.clone(),
            client_streaming: method.client_streaming,
            server_streaming: method.server_streaming,
            access: access.into(),
            scope,
            request_schema: schema(&method.request_type),
            response_schema: schema(&method.response_type),
            ..Default::default()
        }
    }

    /// Every service (of one domain, or all), without schemas.
    pub fn services(&self, domain: &str) -> Vec<ApiService> {
        self.services
            .iter()
            .filter(|service| domain.is_empty() || service.domain == domain)
            .map(|service| ApiService {
                name: service.name.clone(),
                domain: service.domain.clone(),
                description: service.description.clone(),
                methods: service.methods.iter().map(|method| self.method_proto(service, method, false)).collect(),
                ..Default::default()
            })
            .collect()
    }

    /// One method by its path (`/pkg.Service/Method`, slash optional), with schemas.
    pub fn method(&self, path: &str) -> Option<ApiMethod> {
        let (service, method) = path.strip_prefix('/').unwrap_or(path).split_once('/')?;
        let service = self.services.iter().find(|s| s.name == service)?;
        let method = service.methods.iter().find(|m| m.name == method)?;
        Some(self.method_proto(service, method, true))
    }
}

fn describe(mut schema: Value, description: &str) -> Value {
    if !description.is_empty() {
        // A bare `$ref` may carry siblings in 2020-12, so this is always valid.
        schema["description"] = json!(description);
    }
    schema
}

/// Reference requests one address may make per minute. The reference is
/// open to anyone, and its answers are large.
const REQUESTS_PER_MINUTE: u32 = 120;

impl ReferenceService for Dev {
    async fn list_services(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListServicesRequest>,
    ) -> ServiceResult<ListServicesResponse> {
        handle(async {
            self.core
                .cache
                .rate_limit(
                    &format!("dev-reference:{}", ctx.client().ip),
                    REQUESTS_PER_MINUTE,
                    std::time::Duration::from_secs(60),
                )
                .await?;
            ok(ListServicesResponse { services: self.reference.services(request.domain.trim()), ..Default::default() })
        })
        .await
    }

    async fn get_method(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetMethodRequest>,
    ) -> ServiceResult<GetMethodResponse> {
        handle(async {
            self.core
                .cache
                .rate_limit(
                    &format!("dev-reference:{}", ctx.client().ip),
                    REQUESTS_PER_MINUTE,
                    std::time::Duration::from_secs(60),
                )
                .await?;
            let method = self.reference.method(request.path.trim()).or_not_found("METHOD_NOT_FOUND", "no such RPC")?;
            ok(GetMethodResponse { method: method.into(), ..Default::default() })
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reference() -> Reference {
        Reference::load(tokuly_proto::FILE_DESCRIPTOR_SET).expect("the embedded descriptor set loads")
    }

    #[test]
    fn services_carry_their_proto_comments_and_token_access() {
        let reference = reference();
        let all = reference.services("");
        assert!(all.len() > 20, "the whole API is described, not only this domain");
        assert!(all.iter().all(|s| s.name.starts_with("tokuly.") && !s.methods.is_empty()));
        let dev = reference.services("dev");
        assert_eq!(
            dev.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
            [
                "tokuly.dev.v1.AppService",
                "tokuly.dev.v1.AuthorizationService",
                "tokuly.dev.v1.BotService",
                "tokuly.dev.v1.ReferenceService",
                "tokuly.dev.v1.TokenService",
                "tokuly.dev.v1.UsageService",
                "tokuly.dev.v1.WebhookService",
            ]
        );
        let tokens = dev.iter().find(|s| s.name.ends_with("TokenService")).unwrap();
        assert!(tokens.description.starts_with("Personal access tokens"), "{}", tokens.description);
        let create = tokens.methods.iter().find(|m| m.name == "CreateToken").unwrap();
        assert!(create.description.contains("TOO_MANY_TOKENS"));
        assert_eq!(
            (create.path.as_str(), create.request_type.as_str()),
            ("/tokuly.dev.v1.TokenService/CreateToken", "tokuly.dev.v1.CreateTokenRequest")
        );
        assert_eq!(create.access.as_known(), Some(ApiAccess::API_ACCESS_SESSION_ONLY));
        assert!(create.request_schema.is_empty(), "listings carry no schemas");
        let current = tokens.methods.iter().find(|m| m.name == "GetCurrentToken").unwrap();
        assert_eq!(current.access.as_known(), Some(ApiAccess::API_ACCESS_ANY_TOKEN));
        let webhooks = dev.iter().find(|s| s.name.ends_with("WebhookService")).unwrap();
        let scopes: Vec<(&str, &str)> = webhooks.methods.iter().map(|m| (m.name.as_str(), m.scope.as_str())).collect();
        assert!(scopes.contains(&("CreateWebhook", "")) && scopes.contains(&("ListWebhooks", "dev:read")));
        // An RPC of one domain that carries another's data names both scopes.
        assert_eq!(
            reference.method("/tokuly.feed.v1.MapStoryService/ListMapStories").unwrap().scope,
            "feed:read geo:read"
        );
        // Location RPCs name the location scope; streams are marked.
        let geo = reference.method("tokuly.geo.v1.LocationService/WatchFriendLocations").unwrap();
        assert_eq!((geo.scope.as_str(), geo.server_streaming, geo.client_streaming), ("geo:read", true, false));
    }

    /// The access table in `tokuly_core::api_access` lists every RPC by
    /// hand and refuses tokens anything it does not list. This walks the
    /// API's own descriptor set, so an RPC added to a proto file without a
    /// line in the table — and a line left behind by an RPC that is gone —
    /// fails here, with the name of what to look at.
    #[test]
    fn every_rpc_of_the_api_is_classified_and_nothing_else() {
        let described: Vec<String> = reference()
            .services("")
            .into_iter()
            .flat_map(|service| service.methods)
            .map(|method| method.path)
            .collect();
        assert!(described.len() > 600, "the descriptor set holds the whole API");
        let unclassified: Vec<&String> = described.iter().filter(|path| access_for(path).is_none()).collect();
        assert!(
            unclassified.is_empty(),
            "add these RPCs to tokuly_core::api_access after looking at what they read and write: {unclassified:?}"
        );
        let stale: Vec<String> = tokuly_core::api_access::classified()
            .map(|(path, _)| path)
            .filter(|path| !described.contains(path))
            .collect();
        assert!(stale.is_empty(), "these lines of tokuly_core::api_access name RPCs that do not exist: {stale:?}");
    }

    #[test]
    fn actions_with_a_reading_name_need_the_write_scope_or_a_session() {
        for (path, scope) in [
            ("/tokuly.geo.v1.PlaceService/CheckIn", "geo:write"),
            ("/tokuly.media.v1.MediaService/GetUploadParts", "media:write"),
        ] {
            assert_eq!(reference().method(path).unwrap().scope, scope, "{path}");
        }
        let sos = reference().method("/tokuly.geo.v1.SafetyService/ResolveSos").unwrap();
        assert_eq!(sos.access.as_known(), Some(ApiAccess::API_ACCESS_SESSION_ONLY));
    }

    #[test]
    fn a_method_comes_with_self_contained_json_schemas() {
        let reference = reference();
        let method = reference.method("/tokuly.dev.v1.TokenService/CreateToken").unwrap();
        let request: Value = serde_json::from_str(&method.request_schema).unwrap();
        assert_eq!(request["$schema"], "https://json-schema.org/draft/2020-12/schema");
        assert_eq!(request["$ref"], "#/$defs/tokuly.dev.v1.CreateTokenRequest");
        let message = &request["$defs"]["tokuly.dev.v1.CreateTokenRequest"];
        assert_eq!(message["properties"]["name"], json!({ "type": "string", "description": "1-60 characters." }));
        assert_eq!(message["properties"]["scopes"]["type"], "array");
        assert_eq!(message["properties"]["expiry"]["$ref"], "#/$defs/tokuly.dev.v1.TokenExpiry");
        let expiry = &request["$defs"]["tokuly.dev.v1.TokenExpiry"];
        assert_eq!(expiry["enum"][5], "TOKEN_EXPIRY_NEVER");
        assert_eq!(expiry["x-enum-descriptions"]["TOKEN_EXPIRY_NEVER"], "Valid until revoked.");

        let response: Value = serde_json::from_str(&method.response_schema).unwrap();
        let token = &response["$defs"]["tokuly.dev.v1.AccessToken"]["properties"];
        // JSON names, and the JSON form of well-known types.
        assert_eq!(token["createdAt"]["type"], "string");
        assert_eq!(token["createdAt"]["format"], "date-time");
        assert_eq!(token["lastUsedIp"]["type"], "string");
        // Every reference resolves inside the document.
        for document in [&request, &response] {
            let text = document.to_string();
            for reference in text.split("#/$defs/").skip(1) {
                let name = reference.split('"').next().unwrap();
                assert!(document["$defs"].get(name).is_some(), "{name} is defined");
            }
        }
        assert!(
            reference.method("/tokuly.dev.v1.TokenService/Nope").is_none() && reference.method("nonsense").is_none()
        );
    }

    #[test]
    fn oneofs_maps_and_nested_messages_are_described() {
        let reference = reference();
        let consent = reference.schema("tokuly.dev.v1.GetConsentResponse");
        let message = &consent["$defs"]["tokuly.dev.v1.GetConsentResponse"];
        assert_eq!(message["x-oneofs"]["result"], json!(["consent", "redirectUrl"]));
        assert!(
            consent["$defs"].get("tokuly.common.v1.UserSummary").is_some(),
            "definitions are followed transitively"
        );
        // proto3 `optional` is not a oneof to JSON readers.
        let update = reference.schema("tokuly.dev.v1.UpdateBotRequest");
        assert!(update["$defs"]["tokuly.dev.v1.UpdateBotRequest"].get("x-oneofs").is_none());
        // 64-bit integers are strings in JSON.
        let usage = reference.schema("tokuly.dev.v1.GetUsageResponse");
        assert_eq!(usage["$defs"]["tokuly.dev.v1.GetUsageResponse"]["properties"]["requests"]["format"], "int64");
        // A map field becomes an object.
        let me = reference.schema("tokuly.identity.v1.Me");
        assert_eq!(
            me["$defs"]["tokuly.identity.v1.Me"]["properties"]["features"]["additionalProperties"]["type"],
            "boolean"
        );
    }
}
