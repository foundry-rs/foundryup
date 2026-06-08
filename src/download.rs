use crate::warn;
use eyre::{Result, WrapErr, bail};
use fs_err as fs;
use futures_util::StreamExt;
use indicatif::{ProgressBar, ProgressStyle};
use sha2::{Digest, Sha256};
use std::{
    io::Write,
    path::Path,
    time::{Duration, Instant},
};

/// Number of retries (after the initial attempt) for transient HTTP failures.
const MAX_RETRIES: u32 = 5;
/// Delay before the first retry. Doubles on each subsequent retry.
const INITIAL_RETRY_DELAY: Duration = Duration::from_secs(2);
/// Upper bound for a single backoff delay.
const MAX_RETRY_DELAY: Duration = Duration::from_secs(10);
/// Overall time budget for retries, after which we give up.
const MAX_RETRY_ELAPSED: Duration = Duration::from_secs(60);

/// Transient HTTP statuses that may recover on retry (e.g. GitHub rate limiting
/// or temporary outages). Other errors (e.g. 404) are treated as permanent.
fn is_retryable_status(status: reqwest::StatusCode) -> bool {
    matches!(status.as_u16(), 403 | 408 | 429 | 500 | 502 | 503 | 504)
}

pub(crate) struct Downloader {
    client: reqwest::Client,
    /// Delay before the first retry; configurable so tests can run fast.
    retry_delay: Duration,
}

impl Downloader {
    pub(crate) fn new() -> Result<Self> {
        let client = reqwest::Client::builder()
            .user_agent(concat!("foundryup/", env!("CARGO_PKG_VERSION")))
            .build()
            .wrap_err("failed to create HTTP client")?;
        Ok(Self { client, retry_delay: INITIAL_RETRY_DELAY })
    }

    /// Sends a GET request, retrying on transient failures with exponential
    /// backoff. Returns the successful response, or an error once retries or the
    /// overall time budget are exhausted.
    async fn send_with_retry(&self, url: &str) -> Result<reqwest::Response> {
        let start = Instant::now();
        let mut delay = self.retry_delay;

        for retry in 0..=MAX_RETRIES {
            let reason = match self.client.get(url).send().await {
                Ok(response) => {
                    let status = response.status();
                    if status.is_success() {
                        return Ok(response);
                    }
                    // Permanent errors (e.g. 404) won't recover; fail fast.
                    if !is_retryable_status(status) {
                        bail!("failed to download {url}: HTTP {status}");
                    }
                    format!("HTTP {status}")
                }
                Err(e) => e.to_string(),
            };

            // Stop if retries are exhausted or the next backoff would exceed the budget.
            if retry == MAX_RETRIES || start.elapsed() + delay >= MAX_RETRY_ELAPSED {
                bail!("failed to download {url} after {} attempts: {reason}", retry + 1);
            }

            warn!(
                "request to {url} failed ({reason}); retrying in {}s ({}/{MAX_RETRIES})",
                delay.as_secs().max(1),
                retry + 1
            );
            tokio::time::sleep(delay).await;
            delay = (delay * 2).min(MAX_RETRY_DELAY);
        }

        unreachable!("retry loop returns before exhausting the range")
    }

    pub(crate) async fn download_to_file(&self, url: &str, path: &Path) -> Result<()> {
        let response = self.send_with_retry(url).await?;

        let total_size = response.content_length();

        let pb = match total_size {
            Some(size) => {
                let pb = ProgressBar::new(size);
                pb.set_style(
                    ProgressStyle::default_bar()
                        .template(
                            "{spinner:.green} [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})",
                        )
                        .unwrap()
                        .progress_chars("#>-"),
                );
                pb
            }
            None => {
                let pb = ProgressBar::new_spinner();
                pb.set_style(
                    ProgressStyle::default_spinner().template("{spinner:.green} {bytes}").unwrap(),
                );
                pb
            }
        };

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = fs::File::create(path)?;
        let mut stream = response.bytes_stream();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.wrap_err("failed to read response chunk")?;
            file.write_all(&chunk)?;
            pb.inc(chunk.len() as u64);
        }

        pb.finish_and_clear();
        Ok(())
    }

    pub(crate) async fn download_to_string(&self, url: &str) -> Result<String> {
        let response = self.send_with_retry(url).await?;
        response.text().await.wrap_err("failed to read response body")
    }
}

pub(crate) fn compute_sha256(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher)?;
    Ok(hex::encode(hasher.finalize()))
}

pub(crate) fn extract_tar_gz(archive_path: &Path, dest_dir: &Path) -> Result<()> {
    let file = fs::File::open(archive_path)?;
    let gz = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(gz);
    fs::create_dir_all(dest_dir)?;
    archive.unpack(dest_dir)?;
    Ok(())
}

pub(crate) fn extract_zip(archive_path: &Path, dest_dir: &Path) -> Result<()> {
    let file = fs::File::open(archive_path)?;
    let mut archive = zip::ZipArchive::new(file)?;
    fs::create_dir_all(dest_dir)?;

    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let outpath = match file.enclosed_name() {
            Some(path) => dest_dir.join(path),
            None => continue,
        };

        if file.is_dir() {
            fs::create_dir_all(&outpath)?;
        } else {
            if let Some(p) = outpath.parent() {
                fs::create_dir_all(p)?;
            }
            let mut outfile = fs::File::create(&outpath)?;
            std::io::copy(&mut file, &mut outfile)?;
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Some(mode) = file.unix_mode() {
                fs::set_permissions(&outpath, std::fs::Permissions::from_mode(mode))?;
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::Read,
        net::TcpListener,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };

    fn status_text(code: u16) -> &'static str {
        match code {
            200 => "OK",
            404 => "Not Found",
            503 => "Service Unavailable",
            _ => "Status",
        }
    }

    /// Spawns a throwaway HTTP server that answers each connection with the next
    /// status code in `responses`, then returns its base URL and a counter of
    /// how many requests it served.
    fn mock_server(responses: Vec<u16>) -> (String, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let served = Arc::new(AtomicUsize::new(0));
        let served_thread = served.clone();

        std::thread::spawn(move || {
            for code in responses {
                let (mut stream, _) = match listener.accept() {
                    Ok(conn) => conn,
                    Err(_) => break,
                };

                // Consume the request headers.
                let mut buf = Vec::new();
                let mut tmp = [0u8; 1024];
                loop {
                    match stream.read(&mut tmp) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            buf.extend_from_slice(&tmp[..n]);
                            if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                                break;
                            }
                        }
                    }
                }

                let body = if code == 200 { "v1.7.1" } else { "" };
                let response = format!(
                    "HTTP/1.1 {code} {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    status_text(code),
                    body.len(),
                );
                use std::io::Write as _;
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
                served_thread.fetch_add(1, Ordering::SeqCst);
            }
        });

        (format!("http://{addr}/"), served)
    }

    fn downloader() -> Downloader {
        Downloader {
            client: reqwest::Client::builder().build().unwrap(),
            retry_delay: Duration::from_millis(1),
        }
    }

    fn block_on<F: std::future::Future>(f: F) -> F::Output {
        tokio::runtime::Runtime::new().unwrap().block_on(f)
    }

    #[test]
    fn retries_transient_status_then_succeeds() {
        let (url, served) = mock_server(vec![503, 503, 200]);
        let downloader = downloader();

        let body = block_on(downloader.download_to_string(&url)).unwrap();

        assert_eq!(body, "v1.7.1");
        assert_eq!(served.load(Ordering::SeqCst), 3, "should retry until success");
    }

    #[test]
    fn does_not_retry_permanent_status() {
        let (url, served) = mock_server(vec![404]);
        let downloader = downloader();

        let result = block_on(downloader.download_to_string(&url));

        assert!(result.is_err(), "404 should fail");
        assert_eq!(served.load(Ordering::SeqCst), 1, "404 should not be retried");
    }

    #[test]
    fn gives_up_after_max_retries() {
        // One more than MAX_RETRIES + 1 initial attempt, so it is never enough.
        let (url, served) = mock_server(vec![503; (MAX_RETRIES + 2) as usize]);
        let downloader = downloader();

        let result = block_on(downloader.download_to_string(&url));

        assert!(result.is_err(), "persistent 503 should fail");
        assert_eq!(
            served.load(Ordering::SeqCst),
            (MAX_RETRIES + 1) as usize,
            "should attempt exactly MAX_RETRIES + 1 times"
        );
    }

    #[test]
    fn retryable_status_classification() {
        for code in [403, 408, 429, 500, 502, 503, 504] {
            assert!(is_retryable_status(reqwest::StatusCode::from_u16(code).unwrap()));
        }
        for code in [200, 301, 400, 401, 404, 410] {
            assert!(!is_retryable_status(reqwest::StatusCode::from_u16(code).unwrap()));
        }
    }
}
