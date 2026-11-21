//! `tokuly healthcheck <url>`: the container health probe.
//!
//! The runtime image carries no curl or wget, so the binary asks its own
//! health endpoint: exit code 0 when it answers `200`, 1 otherwise.
use std::time::Duration;

use anyhow::Context as _;
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::TcpStream,
};

const TIMEOUT: Duration = Duration::from_secs(3);

pub async fn run(url: &str) -> anyhow::Result<()> {
    let (authority, path) = split(url)?;
    let status_line = tokio::time::timeout(TIMEOUT, async {
        let mut stream = TcpStream::connect(authority).await?;
        stream.write_all(format!("GET {path} HTTP/1.1\r\nHost: {authority}\r\nConnection: close\r\n\r\n").as_bytes()).await?;
        let mut head = Vec::new();
        stream.take(64).read_to_end(&mut head).await?;
        std::io::Result::Ok(String::from_utf8_lossy(&head).lines().next().unwrap_or_default().to_owned())
    })
    .await
    .with_context(|| format!("{url} did not answer within {TIMEOUT:?}"))?
    .with_context(|| format!("cannot reach {url}"))?;
    anyhow::ensure!(status_line.split_whitespace().nth(1) == Some("200"), "{url} answered `{status_line}`");
    Ok(())
}

/// `http://host:port/path` → (`host:port`, `/path`).
fn split(url: &str) -> anyhow::Result<(&str, &str)> {
    let rest = url.strip_prefix("http://").with_context(|| format!("`{url}` is not an http:// URL"))?;
    let slash = rest.find('/').unwrap_or(rest.len());
    let (authority, path) = rest.split_at(slash);
    anyhow::ensure!(authority.contains(':'), "`{url}` needs an explicit port");
    Ok((authority, if path.is_empty() { "/" } else { path }))
}

#[cfg(test)]
mod tests {
    use tokio::net::TcpListener;

    use super::*;

    #[test]
    fn urls_are_split_into_address_and_path() {
        assert_eq!(split("http://127.0.0.1:8081/readyz").unwrap(), ("127.0.0.1:8081", "/readyz"));
        assert_eq!(split("http://localhost:8082").unwrap(), ("localhost:8082", "/"));
        assert!(split("https://localhost:8081/readyz").is_err());
        assert!(split("http://localhost/readyz").is_err());
    }

    /// A server that answers every request with the given status line.
    async fn answering(status: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let mut request = [0u8; 512];
                let _ = stream.read(&mut request).await;
                let _ = stream.write_all(format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes()).await;
            }
        });
        format!("http://{address}/readyz")
    }

    #[tokio::test]
    async fn only_a_200_counts_as_healthy() {
        assert!(run(&answering("200 OK").await).await.is_ok());
        let unavailable = run(&answering("503 Service Unavailable").await).await.unwrap_err();
        assert!(unavailable.to_string().contains("503"), "{unavailable}");
    }

    #[tokio::test]
    async fn an_unreachable_service_is_unhealthy() {
        // Bind, then free, a port so nothing is listening on it.
        let address = TcpListener::bind("127.0.0.1:0").await.unwrap().local_addr().unwrap();
        assert!(run(&format!("http://{address}/readyz")).await.is_err());
    }
}
