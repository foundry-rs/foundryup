use digest_io::IoWrapper;
use eyre::{Result, WrapErr, bail};
use fs_err as fs;
use futures_util::StreamExt;
use indicatif::{ProgressBar, ProgressStyle};
use sha2::{Digest, Sha256};
use std::{io::Write, path::Path};

/// Default number of retries (after the initial attempt) for transient HTTP
/// failures, used when `FOUNDRYUP_MAX_RETRIES` is unset or unparsable.
const DEFAULT_MAX_RETRIES: u32 = 5;

/// Number of retries for transient HTTP failures, honoring the
/// `FOUNDRYUP_MAX_RETRIES` environment variable (matching the install script).
///
/// The script's `FOUNDRYUP_RETRY_DELAY` / `FOUNDRYUP_RETRY_MAX_TIME` are not
/// supported here: reqwest's retry layer manages its own backoff and does not
/// expose delay or total-time knobs.
pub(crate) fn max_retries() -> u32 {
    static CACHE: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *CACHE.get_or_init(|| {
        std::env::var("FOUNDRYUP_MAX_RETRIES")
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(DEFAULT_MAX_RETRIES)
    })
}

/// Transient HTTP statuses that may recover on retry (e.g. GitHub rate limiting
/// or temporary outages). Other errors (e.g. 404) are treated as permanent.
fn is_retryable_status(status: reqwest::StatusCode) -> bool {
    matches!(status.as_u16(), 403 | 408 | 429 | 500 | 502 | 503 | 504)
}

fn is_file_not_found(status: reqwest::StatusCode) -> bool {
    status == reqwest::StatusCode::NOT_FOUND
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FileDownload {
    Downloaded,
    NotFound,
}

impl Downloader {
    pub(crate) fn new() -> Result<Self> {
        // `no_budget` disables reqwest's default token budget, which would
        // otherwise block retries on a CLI that issues only a few requests.
        let retry = reqwest::retry::for_host(GitHubHosts)
            .no_budget()
            .max_retries_per_request(max_retries())
            .classify_fn(|req_rep| {
                if req_rep.error().is_some() || req_rep.status().is_some_and(is_retryable_status) {
                    req_rep.retryable()
                } else {
                    req_rep.success()
                }
            });

        let client = reqwest::Client::builder()
            .https_only(true)
            .user_agent(concat!("foundryup/", env!("CARGO_PKG_VERSION")))
            .retry(retry)
            .build()
            .wrap_err("failed to create HTTP client")?;
        Ok(Self { client })
    }

    async fn send(&self, url: &str) -> Result<reqwest::Response> {
        let parsed = reqwest::Url::parse(url).wrap_err_with(|| format!("invalid URL {url}"))?;
        // Only attach the token to GitHub API requests, never to release-download
        // CDN hosts. reqwest also strips it on cross-host redirects.
        let is_github_api = is_github_api_url(&parsed);
        let mut request = self.client.get(parsed);
        if is_github_api {
            if let Some(token) = github_token() {
                request = request.bearer_auth(token);
            }
        }
        let response = request.send().await.wrap_err_with(|| format!("failed to GET {url}"))?;
        Ok(response)
    }

    /// Sends a request and errors on any non-success HTTP status.
    async fn send_ok(&self, url: &str) -> Result<reqwest::Response> {
        let response = self.send(url).await?;
        if !response.status().is_success() {
            bail!("failed to download {url}: HTTP {}", response.status());
        }
        Ok(response)
    }

    pub(crate) async fn download_to_file(&self, url: &str, path: &Path) -> Result<()> {
        match self.download_to_file_optional(url, path).await? {
            FileDownload::Downloaded => Ok(()),
            FileDownload::NotFound => {
                bail!("failed to download {url}: HTTP 404 Not Found");
            }
        }
    }

    /// Downloads `url` to `path`, returning [`FileDownload::NotFound`] only for
    /// a terminal HTTP 404 response.
    ///
    /// All transport failures and other non-success statuses remain errors so
    /// callers can distinguish an absent release asset from a broader GitHub or
    /// network failure. A successful response body is streamed exactly once.
    pub(crate) async fn download_to_file_optional(
        &self,
        url: &str,
        path: &Path,
    ) -> Result<FileDownload> {
        let response = self.send(url).await?;
        if is_file_not_found(response.status()) {
            return Ok(FileDownload::NotFound);
        }
        if !response.status().is_success() {
            bail!("failed to download {url}: HTTP {}", response.status());
        }

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
        Ok(FileDownload::Downloaded)
    }

    pub(crate) async fn download_to_string(&self, url: &str) -> Result<String> {
        let response = self.send_ok(url).await?;
        response.text().await.wrap_err("failed to read response body")
    }

    /// Follows redirects for `url` (via a `HEAD` request, without downloading a
    /// body) and returns the final effective URL.
    ///
    /// Used to resolve the `releases/latest` web redirect to a concrete
    /// `releases/tag/<tag>` URL without calling the rate-limited GitHub API.
    /// No token is attached, since `github.com` (unlike `api.github.com`) is not
    /// subject to the unauthenticated API rate limit.
    pub(crate) async fn resolve_redirect_url(&self, url: &str) -> Result<String> {
        let parsed = reqwest::Url::parse(url).wrap_err_with(|| format!("invalid URL {url}"))?;
        let response = self
            .client
            .head(parsed)
            .send()
            .await
            .wrap_err_with(|| format!("failed to HEAD {url}"))?;
        if !response.status().is_success() {
            bail!("failed to resolve {url}: HTTP {}", response.status());
        }
        Ok(response.url().to_string())
    }

    /// Like [`download_to_string`](Self::download_to_string), but returns
    /// `Ok(None)` when the server responds with HTTP 404 Not Found.
    ///
    /// Transport failures (DNS/TLS/connection errors) and other non-success
    /// statuses are propagated as errors. A genuinely absent attestation (404) skips verification
    /// while a transport failure aborts the install rather than silently downgrading to
    /// an unverified binary.
    pub(crate) async fn download_to_string_optional(&self, url: &str) -> Result<Option<String>> {
        let response = self.send(url).await?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !response.status().is_success() {
            bail!("failed to download {url}: HTTP {}", response.status());
        }
        let body = response.text().await.wrap_err("failed to read response body")?;
        Ok(Some(body))
    }
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
    fn only_not_found_is_an_absent_file_download() {
        assert!(is_file_not_found(reqwest::StatusCode::NOT_FOUND));
        for code in [400, 401, 403, 408, 410, 429, 500, 502, 503, 504] {
            assert!(!is_file_not_found(reqwest::StatusCode::from_u16(code).unwrap()));
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
