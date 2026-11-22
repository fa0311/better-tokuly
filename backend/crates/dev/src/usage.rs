//! `UsageService`: how much a token, application or bot uses the API.
//!
//! Every request made with an API token is tracked as a `dev.api.request`
//! analytics event (`verify.rs`): the subject is the personal token, the
//! application or the bot, `value` the milliseconds it took, and `props`
//! `{ procedure, error }`. This reads them back from ClickHouse.
use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::prelude::*;
use tokuly_proto::dev::{get_usage_request::Subject, *};

use crate::{Dev, verify::USAGE_EVENT};

fn unavailable(error: clickhouse::error::Error) -> AppError {
    // What the store said stays in the log: it names tables and hosts.
    tracing::warn!(error = %error, "usage query failed");
    AppError::unavailable("ANALYTICS_UNAVAILABLE", "usage statistics are not available right now")
}

/// Usage events have no owner (the zero UUID leads the table's sort key).
const WHERE: &str = "owner_id = toUUID('00000000-0000-0000-0000-000000000000') AND name = ? AND subject_type = ? AND subject_id = toUUID(?) \
     AND at >= now() - toIntervalDay(?)";

impl Dev {
    /// The usage of one subject (`token`, `app` or `bot`) over the last `days`.
    pub(crate) async fn usage(&self, subject_type: &str, subject_id: Uuid, days: i32) -> AppResult<GetUsageResponse> {
        let ch = &self.core.analytics.ch;
        let by_day: Vec<(String, u64, u64)> = ch
            .query(&format!(
                "SELECT toString(toDate(at)) AS day, toUInt64(count()), toUInt64(countIf(JSONExtractString(props, 'error') != ''))
                 FROM events WHERE {WHERE} GROUP BY day ORDER BY day"
            ))
            .bind(USAGE_EVENT)
            .bind(subject_type)
            .bind(subject_id.to_string())
            .bind(days)
            .fetch_all()
            .await
            .map_err(unavailable)?;
        let by_method: Vec<(String, u64, u64, f64)> = ch
            .query(&format!(
                "SELECT JSONExtractString(props, 'procedure') AS procedure, toUInt64(count()) AS requests,
                        toUInt64(countIf(JSONExtractString(props, 'error') != '')), toFloat64(avg(value))
                 FROM events WHERE {WHERE} GROUP BY procedure ORDER BY requests DESC, procedure LIMIT 100"
            ))
            .bind(USAGE_EVENT)
            .bind(subject_type)
            .bind(subject_id.to_string())
            .bind(days)
            .fetch_all()
            .await
            .map_err(unavailable)?;
        Ok(GetUsageResponse {
            requests: by_day.iter().map(|(_, requests, _)| *requests as i64).sum(),
            errors: by_day.iter().map(|(_, _, errors)| *errors as i64).sum(),
            days: by_day
                .into_iter()
                .map(|(date, requests, errors)| UsageDay {
                    date,
                    requests: requests as i64,
                    errors: errors as i64,
                    ..Default::default()
                })
                .collect(),
            methods: by_method
                .into_iter()
                .map(|(procedure, requests, errors, average_ms)| UsageMethod {
                    procedure,
                    requests: requests as i64,
                    errors: errors as i64,
                    average_ms: average_ms.round().clamp(0.0, i32::MAX as f64) as i32,
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        })
    }
}

impl UsageService for Dev {
    async fn get_usage(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetUsageRequest>,
    ) -> ServiceResult<GetUsageResponse> {
        handle(async {
            let user_id = ctx.user_id()?;
            let req = request.to_owned_message();
            // Each call is two aggregations over the analytics store.
            self.core.cache.rate_limit(&format!("dev-usage:{user_id}"), 60, std::time::Duration::from_secs(60)).await?;
            let days = match req.days {
                0 => 30,
                days => days.clamp(1, 90),
            };
            // Only what the caller owns; anything else does not exist.
            let (subject_type, subject_id) = match &req.subject {
                Some(Subject::TokenId(id)) => ("token", self.owned_token(parse_id(id)?, user_id, "pat").await?.id),
                Some(Subject::AppId(id)) => ("app", self.owned_app(parse_id(id)?, user_id).await?.id),
                Some(Subject::BotId(id)) => ("bot", self.owned_bot(parse_id(id)?, user_id).await?.user_id),
                None => bail!(invalid, "USAGE_SUBJECT_REQUIRED", "name a token, an application or a bot"),
            };
            ok(self.usage(subject_type, subject_id, days).await?)
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_the_analytics_store_says_is_not_passed_on() {
        let failure = clickhouse::error::Error::Custom(
            "Code: 60. Table tokuly.events on ch-internal-7:9000 does not exist".to_owned(),
        );
        let error = unavailable(failure);
        assert_eq!((error.code, error.reason.as_ref()), (connectrpc::ErrorCode::Unavailable, "ANALYTICS_UNAVAILABLE"));
        assert_eq!(error.message, "usage statistics are not available right now");
    }
}
