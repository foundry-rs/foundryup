use crate::retry::{max_retries, retry};
use digest_io::IoWrapper;
use eyre::{Result, WrapErr, bail};
use fs_err as fs;
use futures_util::StreamExt;
use indicatif::{ProgressBar, ProgressStyle};
use sha2::{Digest, Sha256};
use std::{future::Future, io::Write, path::Path};

/// Transient HTTP statuses that may recover on retry (e.g. GitHub rate limiting
/// or temporary outages). Other errors (e.g. 404) are treated as permanent.
fn is_retryable_status(status: reqwest::StatusCode) -> bool {
    matches!(status.as_u16(), 403 | 408 | 429 | 500 | 502 | 503 | 504)
}

/// Retry scope matching every GitHub host foundryup talks to, including the
/// CDN hosts that release downloads redirect to.
struct GitHubHosts;

impl PartialEq<&str> for GitHubHosts {
    fn eq(&self, host: &&str) -> bool {
        // Require a `.` boundary so lookalikes like `notgithub.com` don't match.
        let host = host.trim_end_matches('.');
        ["github.com", "githubusercontent.com"].iter().any(|domain| {
            host == *domain || host.strip_suffix(domain).is_some_and(|prefix| prefix.ends_with('.'))
        })
    }
}

/// Returns a GitHub token from the environment, if set, used to authenticate
/// `api.github.com` requests so they use the higher authenticated rate limit.
/// Checks `GITHUB_TOKEN` then `GH_TOKEN`; empty values are ignored.
fn github_token() -> Option<String> {
    static CACHE: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    CACHE
        .get_or_init(|| {
            ["GITHUB_TOKEN", "GH_TOKEN"]
                .into_iter()
                .find_map(|var| std::env::var(var).ok().filter(|t| !t.is_empty()))
        })
        .clone()
}

/// Whether `url` points at the GitHub REST API over HTTPS, used to gate token
/// attachment. Matches the origin exactly (scheme + host + port) so a token is
/// never sent to lookalike hosts like `api.github.com.evil.com`.
fn is_github_api_url(url: &reqwest::Url) -> bool {
    url.scheme() == "https"
        && url.host_str().is_some_and(|host| host.eq_ignore_ascii_case("api.github.com"))
        && url.port_or_known_default() == Some(443)
}

pub(crate) struct Downloader {
    client: reqwest::Client,
    max_retries: u32,
}

impl Downloader {
    pub(crate) fn new() -> Result<Self> {
        let client = reqwest::Client::builder()
            .https_only(true)
            .user_agent(concat!("foundryup/", env!("CARGO_PKG_VERSION")))
            .retry(reqwest::retry::never())
            .build()
            .wrap_err("failed to create HTTP client")?;
        Ok(Self { client, max_retries: max_retries() })
    }

    /// One retry budget covers both the request and consumption of its response body.
    async fn request<T, F, Fut>(&self, method: reqwest::Method, url: &str, consume: F) -> Result<T>
    where
        F: Fn(reqwest::Response) -> Fut,
        Fut: Future<Output = Result<T>>,
    {
        let parsed = reqwest::Url::parse(url).wrap_err_with(|| format!("invalid URL {url}"))?;
        let is_github_api = is_github_api_url(&parsed);
        let mut request = self.client.request(method, parsed);
        // Never attach the token to release downloads or cross-host redirects.
        if is_github_api {
            if let Some(token) = github_token() {
                request = request.bearer_auth(token);
            }
        }
        let request = request.build()?;
        let retryable_host = request.url().host_str().is_some_and(|host| GitHubHosts == host);
        retry(
            self.max_retries,
            || async {
                let response =
                    self.client.execute(request.try_clone().expect("bodyless request")).await?;
                if is_retryable_status(response.status()) {
                    // Drop error bodies before retrying. Consumers handle permanent statuses,
                    // including optional 404s, themselves.
                    response.error_for_status_ref()?;
                }
                consume(response).await
            },
            |error: &eyre::Report| {
                retryable_host
                    && error
                        .downcast_ref::<reqwest::Error>()
                        .is_some_and(|error| error.status().is_none_or(is_retryable_status))
            },
        )
        .await
        .wrap_err_with(|| format!("failed to download {url}"))
    }

    pub(crate) async fn download_to_file(&self, url: &str, path: &Path) -> Result<()> {
        self.request(reqwest::Method::GET, url, |response| async move {
            let response = successful_response(response)?;
            let total_size = response.content_length();

            let pb = match total_size {
                Some(size) => {
                    let pb = ProgressBar::new(size);
                    let template =
                        "{spinner:.green} [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})";
                    pb.set_style(
                        ProgressStyle::default_bar()
                            .template(template)
                            .unwrap()
                            .progress_chars("#>-"),
                    );
                    pb
                }
                None => {
                    let pb = ProgressBar::new_spinner();
                    pb.set_style(
                        ProgressStyle::default_spinner()
                            .template("{spinner:.green} {bytes}")
                            .unwrap(),
                    );
                    pb
                }
            };

            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            // Truncate on every attempt so an interrupted download cannot leave stale bytes.
            let mut file = fs::File::create(path)?;
            let mut stream = response.bytes_stream();

            while let Some(chunk) = stream.next().await {
                let chunk = match chunk {
                    Ok(chunk) => chunk,
                    Err(error) => {
                        pb.finish_and_clear();
                        return Err(error).wrap_err("failed to read response chunk");
                    }
                };
                file.write_all(&chunk)?;
                pb.inc(chunk.len() as u64);
            }

            pb.finish_and_clear();
            Ok(())
        })
        .await
    }

    pub(crate) async fn download_to_string(&self, url: &str) -> Result<String> {
        self.request(reqwest::Method::GET, url, |response| async {
            successful_response(response)?.text().await.wrap_err("failed to read response body")
        })
        .await
    }

    /// Follows redirects for `url` (via a `HEAD` request, without downloading a
    /// body) and returns the final effective URL.
    ///
    /// Used to resolve the `releases/latest` web redirect to a concrete
    /// `releases/tag/<tag>` URL without calling the rate-limited GitHub API.
    /// No token is attached, since `github.com` (unlike `api.github.com`) is not
    /// subject to the unauthenticated API rate limit.
    pub(crate) async fn resolve_redirect_url(&self, url: &str) -> Result<String> {
        self.request(reqwest::Method::HEAD, url, |response| async {
            Ok(successful_response(response)?.url().to_string())
        })
        .await
    }

    /// Returns whether `url` is publicly available without downloading its body.
    ///
    /// A 404 is reported as unavailable. Transport failures and every other
    /// non-success status remain errors so callers do not mistake an outage for
    /// a missing resource.
    pub(crate) async fn is_url_available(&self, url: &str) -> Result<bool> {
        self.request(reqwest::Method::HEAD, url, |response| async {
            if response.status() == reqwest::StatusCode::NOT_FOUND {
                return Ok(false);
            }
            successful_response(response)?;
            Ok(true)
        })
        .await
    }

    /// Like [`download_to_string`](Self::download_to_string), but returns
    /// `Ok(None)` when the server responds with HTTP 404 Not Found.
    ///
    /// Transport failures (DNS/TLS/connection errors) and other non-success
    /// statuses are propagated as errors. A genuinely absent attestation (404) skips verification
    /// while a transport failure aborts the install rather than silently downgrading to
    /// an unverified binary.
    pub(crate) async fn download_to_string_optional(&self, url: &str) -> Result<Option<String>> {
        self.request(reqwest::Method::GET, url, |response| async {
            if response.status() == reqwest::StatusCode::NOT_FOUND {
                return Ok(None);
            }
            let body = successful_response(response)?
                .text()
                .await
                .wrap_err("failed to read response body")?;
            Ok(Some(body))
        })
        .await
    }
}

fn successful_response(response: reqwest::Response) -> Result<reqwest::Response> {
    if !response.status().is_success() {
        bail!("HTTP {}", response.status());
    }
    Ok(response)
}

pub(crate) fn compute_sha256(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = IoWrapper(Sha256::new());
    std::io::copy(&mut file, &mut hasher)?;
    Ok(hex::encode(hasher.0.finalize()))
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

    const UNAVAILABLE: &str =
        "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
    const PARTIAL: &str =
        "HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\nstale partial contents";
    const OK: &str = "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok";
    const MISSING: &str =
        "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";

    // Route a GitHub hostname to a loopback server without TLS or ambient proxies/tokens.
    fn fixture(
        responses: Vec<&'static str>,
        max_retries: u32,
    ) -> (Downloader, String, std::thread::JoinHandle<()>) {
        use std::{
            io::Read,
            net::TcpListener,
            time::{Duration, Instant},
        };
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let server = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(10);
            for response in responses {
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(Instant::now() < deadline, "expected another download attempt");
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(error) => panic!("{error}"),
                    }
                };
                stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    let mut byte = [0];
                    stream.read_exact(&mut byte).unwrap();
                    request.push(byte[0]);
                }
                assert!(
                    !String::from_utf8_lossy(&request)
                        .to_ascii_lowercase()
                        .contains("authorization:")
                );
                stream.write_all(response.as_bytes()).unwrap();
            }
        });
        let client = reqwest::Client::builder()
            .no_proxy()
            .resolve("github.com", address)
            .retry(reqwest::retry::never())
            .build()
            .unwrap();
        (
            Downloader { client, max_retries },
            format!("http://github.com:{}/file", address.port()),
            server,
        )
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap()
    }

    #[test]
    fn file_retries_status_and_partial_body_and_truncates_destination() {
        let (downloader, url, server) = fixture(vec![UNAVAILABLE, PARTIAL, OK], 2);
        let file = tempfile::NamedTempFile::new().unwrap();
        runtime().block_on(downloader.download_to_file(&url, file.path())).unwrap();
        assert_eq!(fs::read(file.path()).unwrap(), b"ok");
        server.join().unwrap();
    }

    #[test]
    fn text_and_optional_text_retry_partial_bodies() {
        for optional in [false, true] {
            let (downloader, url, server) = fixture(vec![PARTIAL, OK], 1);
            let text = runtime().block_on(async {
                if optional {
                    downloader.download_to_string_optional(&url).await.unwrap().unwrap()
                } else {
                    downloader.download_to_string(&url).await.unwrap()
                }
            });
            assert_eq!(text, "ok");
            server.join().unwrap();
        }
    }

    #[test]
    fn head_requests_retry_transient_statuses() {
        for probe in [false, true] {
            let (downloader, url, server) = fixture(vec![UNAVAILABLE, OK], 1);
            runtime().block_on(async {
                if probe {
                    assert!(downloader.is_url_available(&url).await.unwrap());
                } else {
                    assert_eq!(downloader.resolve_redirect_url(&url).await.unwrap(), url);
                }
            });
            server.join().unwrap();
        }
    }

    #[test]
    fn request_and_body_failures_share_one_budget() {
        let (downloader, url, server) = fixture(vec![UNAVAILABLE, PARTIAL], 1);
        let error = runtime().block_on(downloader.download_to_string(&url)).unwrap_err();
        assert!(error.downcast_ref::<reqwest::Error>().unwrap().status().is_none());
        server.join().unwrap();
    }

    #[test]
    fn missing_files_remain_optional_and_local_io_errors_are_permanent() {
        for probe in [false, true] {
            let (downloader, url, server) = fixture(vec![MISSING], 5);
            runtime().block_on(async {
                if probe {
                    assert!(!downloader.is_url_available(&url).await.unwrap());
                } else {
                    assert_eq!(downloader.download_to_string_optional(&url).await.unwrap(), None);
                }
            });
            server.join().unwrap();
        }
        let (downloader, url, server) = fixture(vec![MISSING], 5);
        let error = runtime().block_on(downloader.download_to_string(&url)).unwrap_err();
        assert!(format!("{error:#}").contains("404"));
        server.join().unwrap();

        let (downloader, url, server) = fixture(vec![OK], 5);
        let directory = tempfile::tempdir().unwrap();
        let error =
            runtime().block_on(downloader.download_to_file(&url, directory.path())).unwrap_err();
        assert!(error.downcast_ref::<std::io::Error>().is_some());
        server.join().unwrap();
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

    #[test]
    fn github_api_url_gate_only_matches_exact_origin() {
        let api = |u: &str| is_github_api_url(&reqwest::Url::parse(u).unwrap());
        // Token is attached only for the exact api.github.com HTTPS origin.
        assert!(api("https://api.github.com/repos/x/y/releases/latest"));
        assert!(api("https://API.GITHUB.COM/repos/x/y"));
        assert!(api("https://api.github.com:443/repos/x/y"));
        // Lookalikes, userinfo tricks, other hosts and schemes are rejected.
        assert!(!api("https://api.github.com.evil.com/"));
        assert!(!api("https://api.github.com@evil.com/"));
        assert!(!api("http://api.github.com/"));
        assert!(!api("https://github.com/foundry-rs/foundry/releases/download/v1/x"));
        assert!(!api("https://objects.githubusercontent.com/x"));
    }

    #[test]
    fn github_hosts_scope_matches_github_cdns() {
        assert!(GitHubHosts == "github.com");
        assert!(GitHubHosts == "api.github.com");
        assert!(GitHubHosts == "raw.githubusercontent.com");
        assert!(GitHubHosts == "objects.githubusercontent.com");
        assert!(GitHubHosts != "example.com");
        assert!(GitHubHosts != "notgithub.com");
        assert!(GitHubHosts != "evilgithubusercontent.com");
        assert!(GitHubHosts != "github.com.evil.com");
    }
}
