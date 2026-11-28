//! Logging and tracing setup.
//!
//! Logs go to stdout (pretty for humans, JSON for collectors). When an OTLP
//! endpoint is configured, spans and metrics are also exported to it (the
//! bundled Grafana LGTM stack in the `observability` compose profile).
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::WithExportConfig as _;
use opentelemetry_sdk::{Resource, metrics::SdkMeterProvider, trace::SdkTracerProvider};
use tracing_subscriber::{
    EnvFilter, Layer as _, layer::SubscriberExt as _, util::SubscriberInitExt as _,
};

use crate::config::Config;

/// Keeps the exporters alive; flushes pending spans and metrics on drop.
pub struct TelemetryGuard {
    provider: Option<SdkTracerProvider>,
    meters: Option<SdkMeterProvider>,
}

impl Drop for TelemetryGuard {
    fn drop(&mut self) {
        if let Some(provider) = self.provider.take() {
            let _ = provider.shutdown();
        }
        if let Some(meters) = self.meters.take() {
            let _ = meters.shutdown();
        }
    }
}

/// Exports the metrics recorded through `opentelemetry::global::meter`
/// (see [`crate::backlog::report`]).
fn meter_provider(endpoint: &str, service: &str) -> Option<SdkMeterProvider> {
    match opentelemetry_otlp::MetricExporter::builder().with_tonic().with_endpoint(endpoint.to_owned()).build() {
        Ok(exporter) => {
            let provider = SdkMeterProvider::builder()
                .with_periodic_exporter(exporter)
                .with_resource(Resource::builder().with_service_name(service.to_owned()).build())
                .build();
            opentelemetry::global::set_meter_provider(provider.clone());
            Some(provider)
        }
        Err(e) => {
            eprintln!("telemetry: OTLP metrics disabled ({e})");
            None
        }
    }
}

/// Installs the global tracing subscriber. `service` names this process in
/// traces, e.g. `tokuly-api`.
pub fn init(cfg: &Config, service: &str) -> TelemetryGuard {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new("info,tokuly=debug,tokuly_core=debug,sqlx=warn,async_nats=warn,hyper=warn,h2=warn,aws_smithy_runtime=warn,aws_sdk_s3=warn")
    });

    let fmt_layer = if cfg.log_format == "json" {
        tracing_subscriber::fmt::layer()
            .json()
            .with_current_span(true)
            .boxed()
    } else {
        tracing_subscriber::fmt::layer()
            .compact()
            .with_target(true)
            .boxed()
    };

    let mut provider = None;
    let otel_layer = if cfg.otlp_endpoint.is_empty() {
        None
    } else {
        match opentelemetry_otlp::SpanExporter::builder()
            .with_tonic()
            .with_endpoint(cfg.otlp_endpoint.clone())
            .build()
        {
            Ok(exporter) => {
                let tracer_provider = SdkTracerProvider::builder()
                    .with_batch_exporter(exporter)
                    .with_resource(
                        Resource::builder()
                            .with_service_name(service.to_owned())
                            .build(),
                    )
                    .build();
                let tracer = tracer_provider.tracer("tokuly");
                provider = Some(tracer_provider);
                Some(tracing_opentelemetry::layer().with_tracer(tracer))
            }
            Err(e) => {
                eprintln!("telemetry: OTLP exporter disabled ({e})");
                None
            }
        }
    };

    let meters = (!cfg.otlp_endpoint.is_empty()).then(|| meter_provider(&cfg.otlp_endpoint, service)).flatten();

    tracing_subscriber::registry()
        .with(filter)
        .with(fmt_layer)
        .with(otel_layer)
        .init();
    TelemetryGuard { provider, meters }
}
