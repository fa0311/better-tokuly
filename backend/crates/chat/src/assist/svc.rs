//! `AssistService` (users) and `AssistAdminService` (the operator): thin
//! handlers over [`super`] and [`super::settings`].
use std::time::Duration;

use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::prelude::*;
use tokuly_proto::chat::*;

use super::settings::{self, KeyChange, ProviderUpdate};
use crate::Chat;

/// Own API keys one user may store per hour.
const OWN_KEYS_PER_HOUR: u32 = 10;
const MAX_MODEL_CHARS: usize = 200;

impl AssistService for Chat {
    async fn rewrite_draft(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RewriteDraftRequest>,
    ) -> ServiceResult<RewriteDraftResponse> {
        handle(async {
            let text = Chat::rewrite_draft(self, ctx.user_id()?, request.text, request.tone.as_known()).await?;
            ok(RewriteDraftResponse { text, ..Default::default() })
        })
        .await
    }

    async fn suggest_replies(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SuggestRepliesRequest>,
    ) -> ServiceResult<SuggestRepliesResponse> {
        handle(async {
            let (replies, skipped_messages) =
                Chat::suggest_replies(self, ctx.user_id()?, parse_id(request.conversation_id)?).await?;
            ok(SuggestRepliesResponse { replies, skipped_messages, ..Default::default() })
        })
        .await
    }

    async fn translate_message(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, TranslateMessageRequest>,
    ) -> ServiceResult<TranslateMessageResponse> {
        handle(async {
            let (text, source_language, target_language) =
                Chat::translate_message(self, ctx.user_id()?, parse_id(request.message_id)?, request.target_language)
                    .await?;
            ok(TranslateMessageResponse { text, source_language, target_language, ..Default::default() })
        })
        .await
    }

    async fn get_assist_settings(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, GetAssistSettingsRequest>,
    ) -> ServiceResult<GetAssistSettingsResponse> {
        handle(async {
            let settings = self.ai_settings_view(ctx.user_id()?).await?;
            ok(GetAssistSettingsResponse { settings: settings.into(), ..Default::default() })
        })
        .await
    }

    async fn choose_assist_provider(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ChooseAssistProviderRequest>,
    ) -> ServiceResult<ChooseAssistProviderResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let provider = match request.provider.as_known() {
                None => bail!(invalid, "PROVIDER_INVALID", "unknown AI provider"),
                Some(AiProvider::AI_PROVIDER_UNSPECIFIED) => None,
                Some(provider) => Some(provider),
            };
            if let (Some(provider), false) = (provider, request.off) {
                let instance = self.ai_instance().await?;
                ensure!(
                    instance.get(provider).is_some_and(settings::ProviderSettings::configured),
                    precondition,
                    "PROVIDER_NOT_AVAILABLE",
                    "this AI provider is not offered on this server"
                );
            }
            self.save_ai_choice(user_id, provider, request.off).await?;
            let settings = self.ai_settings_view(user_id).await?;
            ok(ChooseAssistProviderResponse { settings: settings.into(), ..Default::default() })
        })
        .await
    }

    async fn set_own_api_key(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SetOwnApiKeyRequest>,
    ) -> ServiceResult<SetOwnApiKeyResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let provider = settings::known_provider(&request.provider)?;
            ensure!(
                settings::accepts_own_key(provider),
                invalid,
                "PROVIDER_INVALID",
                "only Anthropic and OpenAI take keys of your own"
            );
            let key = settings::check_api_key(request.api_key)?;
            self.core
                .cache
                .rate_limit(&format!("chat-ai-key:{user_id}"), OWN_KEYS_PER_HOUR, Duration::from_secs(3600))
                .await?;
            self.save_own_key(user_id, provider, &key).await?;
            let settings = self.ai_settings_view(user_id).await?;
            ok(SetOwnApiKeyResponse { settings: settings.into(), ..Default::default() })
        })
        .await
    }

    async fn remove_own_api_key(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RemoveOwnApiKeyRequest>,
    ) -> ServiceResult<RemoveOwnApiKeyResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let provider = settings::known_provider(&request.provider)?;
            ensure!(
                settings::accepts_own_key(provider),
                invalid,
                "PROVIDER_INVALID",
                "no key of your own for this provider"
            );
            self.remove_own_key(user_id, provider).await?;
            let settings = self.ai_settings_view(user_id).await?;
            ok(RemoveOwnApiKeyResponse { settings: settings.into(), ..Default::default() })
        })
        .await
    }

    async fn set_ai_message_consent(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SetAiMessageConsentRequest>,
    ) -> ServiceResult<SetAiMessageConsentResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            self.save_ai_consent(user_id, request.allow).await?;
            let settings = self.ai_settings_view(user_id).await?;
            ok(SetAiMessageConsentResponse { settings: settings.into(), ..Default::default() })
        })
        .await
    }

    async fn open_bot_conversation(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, OpenBotConversationRequest>,
    ) -> ServiceResult<OpenBotConversationResponse> {
        handle(async {
            let access = Chat::open_bot_conversation(self, ctx.user_id()?).await?;
            ok(OpenBotConversationResponse {
                conversation: self.conversation_for(&access).await?.into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn set_bot_enabled(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SetBotEnabledRequest>,
    ) -> ServiceResult<SetBotEnabledResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            self.save_bot_enabled(user_id, request.enabled).await?;
            let settings = self.ai_settings_view(user_id).await?;
            ok(SetBotEnabledResponse { settings: settings.into(), ..Default::default() })
        })
        .await
    }
}

impl AssistAdminService for Chat {
    async fn get_assist_instance(
        &self,
        ctx: RequestContext,
        _request: ServiceRequest<'_, GetAssistInstanceRequest>,
    ) -> ServiceResult<GetAssistInstanceResponse> {
        handle(async {
            ctx.require_role("admin")?;
            ok(GetAssistInstanceResponse {
                instance: self.ai_instance().await?.to_proto().into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn update_assist_provider(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, UpdateAssistProviderRequest>,
    ) -> ServiceResult<UpdateAssistProviderResponse> {
        handle(async {
            let admin_id = ctx.require_role("admin")?.user_id;
            let provider = settings::known_provider(&request.provider)?;
            ensure!(
                !request.allow_private_host || provider == AiProvider::AI_PROVIDER_OPENAI_COMPATIBLE,
                invalid,
                "PRIVATE_HOST_NOT_ALLOWED",
                "only the OpenAI-compatible provider may be on a private network"
            );
            let model = request.model.trim();
            ensure!(
                model.chars().count() <= MAX_MODEL_CHARS
                    && !model.chars().any(char::is_control)
                    && (!model.is_empty() || !request.enabled),
                invalid,
                "MODEL_INVALID",
                "name the model (at most {MAX_MODEL_CHARS} characters)"
            );
            let base_url = if request.base_url.trim().is_empty()
                && !request.enabled
                && provider == AiProvider::AI_PROVIDER_OPENAI_COMPATIBLE
            {
                // A compatible provider that is switched off may have no address.
                String::new()
            } else {
                settings::check_base_url(provider, request.base_url, request.allow_private_host, self.ai_policy())?
            };
            let key = match request.api_key_change.as_known() {
                Some(ApiKeyChange::API_KEY_CHANGE_SET) => KeyChange::Set(settings::check_api_key(request.api_key)?),
                Some(ApiKeyChange::API_KEY_CHANGE_CLEAR) => KeyChange::Clear,
                _ => KeyChange::Keep,
            };
            let update = ProviderUpdate {
                provider,
                enabled: request.enabled,
                model: model.to_owned(),
                base_url,
                key,
                allow_private_host: request.allow_private_host,
            };
            self.save_ai_provider(admin_id, update).await?;
            ok(UpdateAssistProviderResponse {
                instance: self.ai_instance().await?.to_proto().into(),
                ..Default::default()
            })
        })
        .await
    }

    async fn set_default_assist_provider(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SetDefaultAssistProviderRequest>,
    ) -> ServiceResult<SetDefaultAssistProviderResponse> {
        handle(async {
            let admin_id = ctx.require_role("admin")?.user_id;
            let provider = match request.provider.as_known() {
                None => bail!(invalid, "PROVIDER_INVALID", "unknown AI provider"),
                Some(AiProvider::AI_PROVIDER_UNSPECIFIED) => None,
                Some(provider) => Some(provider),
            };
            if let Some(provider) = provider {
                let instance = self.ai_instance().await?;
                ensure!(
                    instance.get(provider).is_some_and(settings::ProviderSettings::configured),
                    precondition,
                    "PROVIDER_NOT_AVAILABLE",
                    "enable this provider before making it the default"
                );
            }
            self.save_ai_default(admin_id, provider).await?;
            ok(SetDefaultAssistProviderResponse {
                instance: self.ai_instance().await?.to_proto().into(),
                ..Default::default()
            })
        })
        .await
    }
}
