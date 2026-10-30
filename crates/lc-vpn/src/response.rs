//! Bounded response collection; callers supply errors that redact secret URLs.

use lc_types::{AppError, AppResult};

pub(crate) async fn read_limited(
    mut response: reqwest::Response,
    limit: usize,
    too_large: impl Fn() -> AppError,
    network: impl Fn(reqwest::Error) -> AppError,
) -> AppResult<Vec<u8>> {
    if response.content_length().is_some_and(|n| n > limit as u64) {
        return Err(too_large());
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(&network)? {
        // Count delivered bytes, including decoded content and chunked bodies.
        if chunk.len() > limit.saturating_sub(body.len()) {
            return Err(too_large());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

#[cfg(test)]
pub(crate) mod tests {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use url::Url;

    /// Sends a chunked body, withholding EOF so a late size check would hang.
    pub(crate) async fn oversized_chunked(limit: usize) -> (Url, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            loop {
                let b = socket.read_u8().await.unwrap();
                request.push(b);
                if request.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n")
                .await
                .unwrap();
            let chunk = vec![b'x'; 4096];
            let mut remaining = limit + 1;
            while remaining > 0 {
                let n = remaining.min(chunk.len());
                if socket
                    .write_all(format!("{n:x}\r\n").as_bytes())
                    .await
                    .is_err()
                    || socket.write_all(&chunk[..n]).await.is_err()
                    || socket.write_all(b"\r\n").await.is_err()
                {
                    return;
                }
                remaining -= n;
            }
            // Deliberately never send the final zero chunk.
            std::future::pending::<()>().await;
        });
        (
            Url::parse(&format!("http://{addr}/secret-subscription")).unwrap(),
            server,
        )
    }
}
