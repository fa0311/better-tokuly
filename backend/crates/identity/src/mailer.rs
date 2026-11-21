//! Transactional emails sent by the identity domain.
use tokuly_core::prelude::*;

use crate::Identity;

struct Mail {
    subject: String,
    heading: String,
    body: String,
    button: String,
    url: String,
    footer: String,
}

fn japanese(locale: &str) -> bool {
    locale.to_ascii_lowercase().starts_with("ja")
}

fn render(mail: &Mail) -> (String, String) {
    let text = format!(
        "{}\n\n{}\n\n{}\n\n{}\n",
        mail.heading, mail.body, mail.url, mail.footer
    );
    let html = format!(
        r#"<!doctype html><html><body style="margin:0;background:#f4f2ff;font-family:-apple-system,'Segoe UI','Hiragino Sans','Noto Sans JP',sans-serif;color:#1b1530">
<table role="presentation" width="100%" cellpadding="0" cellspacing="0"><tr><td align="center" style="padding:32px 16px">
<table role="presentation" width="480" cellpadding="0" cellspacing="0" style="max-width:480px;background:#ffffff;border-radius:24px;padding:32px">
<tr><td style="font-size:22px;font-weight:800;letter-spacing:-0.02em;color:#7c5cff">Tokuly</td></tr>
<tr><td style="padding-top:20px;font-size:20px;font-weight:700">{heading}</td></tr>
<tr><td style="padding-top:12px;font-size:15px;line-height:1.7;color:#4a4560">{body}</td></tr>
<tr><td style="padding-top:24px"><a href="{url}" style="display:inline-block;background:#7c5cff;color:#ffffff;text-decoration:none;font-weight:700;padding:14px 24px;border-radius:999px">{button}</a></td></tr>
<tr><td style="padding-top:24px;font-size:12px;line-height:1.6;color:#8a86a0">{footer}</td></tr>
</table></td></tr></table></body></html>"#,
        heading = escape(&mail.heading),
        body = escape(&mail.body),
        url = escape(&mail.url),
        button = escape(&mail.button),
        footer = escape(&mail.footer),
    );
    (text, html)
}

fn escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

impl Identity {
    async fn deliver(&self, to: &str, mail: Mail) -> AppResult<()> {
        let (text, html) = render(&mail);
        self.core.mail.send(to, &mail.subject, &text, &html).await
    }

    pub(crate) async fn send_verification_mail(
        &self,
        to: &str,
        locale: &str,
        token: &str,
        changing: bool,
    ) -> AppResult<()> {
        let url = self.core.web_url(&format!("verify-email?token={token}"));
        let mail = if japanese(locale) {
            Mail {
                subject: "【Tokuly】メールアドレスの確認".into(),
                heading: if changing { "新しいメールアドレスを確認しよう！".into() } else { "Tokulyへようこそ！".into() },
                body: "下のボタンを押して、メールアドレスの確認を完了してください。このリンクは24時間有効です。".into(),
                button: "メールアドレスを確認".into(),
                url,
                footer: "このメールに心当たりがない場合は、何もせずに破棄してください。".into(),
            }
        } else {
            Mail {
                subject: "Confirm your email for Tokuly".into(),
                heading: if changing { "Confirm your new email address".into() } else { "Welcome to Tokuly!".into() },
                body: "Tap the button below to confirm your email address. The link is valid for 24 hours.".into(),
                button: "Confirm email".into(),
                url,
                footer: "If you did not request this, you can safely ignore this email.".into(),
            }
        };
        self.deliver(to, mail).await
    }

    pub(crate) async fn send_password_reset_mail(
        &self,
        to: &str,
        locale: &str,
        token: &str,
    ) -> AppResult<()> {
        let url = self.core.web_url(&format!("reset-password?token={token}"));
        let mail = if japanese(locale) {
            Mail {
                subject: "【Tokuly】パスワードの再設定".into(),
                heading: "パスワードを再設定しよう".into(),
                body: "下のボタンから新しいパスワードを設定できます。このリンクは1時間有効で、一度だけ使えます。".into(),
                button: "パスワードを再設定".into(),
                url,
                footer: "再設定を依頼していない場合は、このメールを無視してください。パスワードは変更されません。".into(),
            }
        } else {
            Mail {
                subject: "Reset your Tokuly password".into(),
                heading: "Reset your password".into(),
                body: "Use the button below to choose a new password. The link is valid for 1 hour and works once.".into(),
                button: "Reset password".into(),
                url,
                footer: "If you did not ask for this, ignore this email and your password stays the same.".into(),
            }
        };
        self.deliver(to, mail).await
    }

    pub(crate) async fn send_data_export_mail(
        &self,
        to: &str,
        locale: &str,
        download_url: &str,
    ) -> AppResult<()> {
        let mail = if japanese(locale) {
            Mail {
                subject: "【Tokuly】データのエクスポートが完了しました".into(),
                heading: "データの準備ができました".into(),
                body: "リクエストされたTokulyのデータをダウンロードできます。このリンクは7日間有効です。".into(),
                button: "データをダウンロード".into(),
                url: download_url.to_owned(),
                footer: "このリンクは他の人と共有しないでください。".into(),
            }
        } else {
            Mail {
                subject: "Your Tokuly data export is ready".into(),
                heading: "Your data is ready".into(),
                body: "The export of your Tokuly data can be downloaded now. The link is valid for 7 days.".into(),
                button: "Download data".into(),
                url: download_url.to_owned(),
                footer: "Do not share this link with anyone.".into(),
            }
        };
        self.deliver(to, mail).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_escapes_dynamic_values() {
        let mail = Mail {
            subject: "s".into(),
            heading: "<b>hi</b>".into(),
            body: "a & b".into(),
            button: "go".into(),
            url: "https://x.test/?a=1&b=\"2\"".into(),
            footer: "f".into(),
        };
        let (text, html) = render(&mail);
        assert!(html.contains("&lt;b&gt;hi&lt;/b&gt;") && html.contains("a &amp; b"));
        assert!(html.contains("https://x.test/?a=1&amp;b=&quot;2&quot;"));
        assert!(text.contains("https://x.test/?a=1&b=\"2\""));
    }
}
