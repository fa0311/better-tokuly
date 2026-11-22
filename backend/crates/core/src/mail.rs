//! Outgoing email over SMTP (Mailpit in development).
use lettre::{
    AsyncSmtpTransport, AsyncTransport as _, Message, Tokio1Executor,
    message::{Mailbox, MultiPart},
};

use crate::error::{AppError, AppResult};

#[derive(Clone)]
pub struct Mailer {
    transport: Option<AsyncSmtpTransport<Tokio1Executor>>,
    from: Mailbox,
}

impl Mailer {
    /// `smtp_url` empty disables sending; mails are logged instead.
    pub fn new(smtp_url: &str, from: &str) -> anyhow::Result<Self> {
        let from: Mailbox =
            from.parse().map_err(|e| anyhow::anyhow!("TOKULY_MAIL_FROM is not a valid mailbox: {e}"))?;
        let transport = if smtp_url.is_empty() {
            None
        } else {
            Some(
                AsyncSmtpTransport::<Tokio1Executor>::from_url(smtp_url)
                    .map_err(|e| anyhow::anyhow!("TOKULY_SMTP_URL is invalid: {e}"))?
                    .build(),
            )
        };
        Ok(Self { transport, from })
    }

    pub fn enabled(&self) -> bool {
        self.transport.is_some()
    }

    /// Sends a mail with both plain-text and HTML bodies.
    pub async fn send(&self, to: &str, subject: &str, text: &str, html: &str) -> AppResult<()> {
        let Some(transport) = &self.transport else {
            tracing::info!(to, subject, body = text, "mail delivery disabled; message logged");
            return Ok(());
        };
        let to: Mailbox = to.parse().map_err(|_| AppError::invalid("INVALID_EMAIL", "invalid email address"))?;
        let message = Message::builder()
            .from(self.from.clone())
            .to(to)
            .subject(subject)
            .multipart(MultiPart::alternative_plain_html(text.to_owned(), html.to_owned()))
            .map_err(|e| AppError::internal(anyhow::anyhow!("mail build: {e}")))?;
        transport.send(message).await.map_err(|e| AppError::internal(anyhow::anyhow!("mail send: {e}")))?;
        Ok(())
    }
}
