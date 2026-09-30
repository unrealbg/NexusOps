use base64::{Engine as _, engine::general_purpose::STANDARD};
use minisign_verify::{PublicKey, Signature, StreamVerifier};
use reqwest::{StatusCode, Url, redirect};
use semver::Version;
use std::{
    future::Future,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
    time::Duration,
};

pub(crate) const MAX_ARTIFACT_BYTES: usize = 134_217_728;
pub(crate) const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
pub(crate) const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(300);
pub(crate) const MAX_REDIRECTS: usize = 3;
const MAX_SIGNATURE_TEXT_BYTES: usize = 4_096;
const MAX_VERSION_BYTES: usize = 128;
const INITIAL_HOST: &str = "github.com";
const INITIAL_PATH_PREFIX: &str = "/unrealbg/NexusOps/releases/download/";
const REDIRECT_HOSTS: [&str; 3] = [
    "github.com",
    "release-assets.githubusercontent.com",
    "objects.githubusercontent.com",
];
const USER_AGENT: &str = "NexusOps/0.1 updater";
const PRODUCTION_PUBLIC_KEY: &str =
    include_str!("../../../../docs/release/keys/nexusops-updater.pub");

pub(crate) const PHASE_DOWNLOADING: u8 = 0;
pub(crate) const PHASE_VERIFYING: u8 = 1;

#[derive(Clone)]
pub(crate) struct PendingAnnouncement {
    pub(crate) version: String,
    pub(crate) url: Url,
    pub(crate) signature: String,
}

impl PendingAnnouncement {
    pub(crate) fn new(
        version: String,
        url: Url,
        signature: String,
    ) -> Result<Self, DownloadFailure> {
        validate_version(&version)?;
        validate_initial_url(&url)?;
        decode_signature(&signature)?;
        Ok(Self {
            version,
            url,
            signature,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DownloadFailure {
    Download,
    Verification,
    ResourceLimit,
    Timeout,
    Cancelled,
}

pub(crate) type DownloadFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Vec<u8>, DownloadFailure>> + Send + 'a>>;

pub(crate) trait ArtifactDownloader: Send + Sync {
    fn download<'a>(
        &'a self,
        announcement: PendingAnnouncement,
        phase: Arc<AtomicU8>,
    ) -> DownloadFuture<'a>;
}

#[derive(Default)]
pub(crate) struct BoundedArtifactDownloader;

impl ArtifactDownloader for BoundedArtifactDownloader {
    fn download<'a>(
        &'a self,
        announcement: PendingAnnouncement,
        phase: Arc<AtomicU8>,
    ) -> DownloadFuture<'a> {
        Box::pin(async move {
            with_total_timeout(
                DOWNLOAD_TIMEOUT,
                download_and_verify(announcement, phase, PRODUCTION_PUBLIC_KEY),
            )
            .await
        })
    }
}

async fn with_total_timeout<F>(duration: Duration, future: F) -> Result<Vec<u8>, DownloadFailure>
where
    F: Future<Output = Result<Vec<u8>, DownloadFailure>>,
{
    tokio::time::timeout(duration, future)
        .await
        .map_err(|_| DownloadFailure::Timeout)?
}

async fn download_and_verify(
    announcement: PendingAnnouncement,
    phase: Arc<AtomicU8>,
    encoded_public_key: &str,
) -> Result<Vec<u8>, DownloadFailure> {
    validate_initial_url(&announcement.url)?;
    let public_key = decode_public_key(encoded_public_key)?;
    let signature = decode_signature(&announcement.signature)?;
    let verifier = public_key
        .verify_stream(&signature)
        .map_err(|_| DownloadFailure::Verification)?;

    let client = reqwest::Client::builder()
        .https_only(true)
        .no_proxy()
        .retry(reqwest::retry::never())
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(DOWNLOAD_TIMEOUT)
        .redirect(redirect_policy())
        .user_agent(USER_AGENT)
        .build()
        .map_err(map_reqwest)?;
    let mut response = client
        .get(announcement.url)
        .send()
        .await
        .map_err(map_reqwest)?;
    validate_status(response.status())?;

    let mut collector =
        BoundedCollector::new(verifier, response.content_length(), MAX_ARTIFACT_BYTES)?;
    while let Some(chunk) = response.chunk().await.map_err(map_reqwest)? {
        collector.accept(&chunk)?;
    }

    phase.store(PHASE_VERIFYING, Ordering::Release);
    let (bytes, trusted_comment) = collector.finish(&signature)?;
    verify_signed_version(trusted_comment, &announcement.version)?;
    Ok(bytes)
}

fn map_reqwest(error: reqwest::Error) -> DownloadFailure {
    if error.is_timeout() {
        DownloadFailure::Timeout
    } else {
        DownloadFailure::Download
    }
}

fn validate_status(status: StatusCode) -> Result<(), DownloadFailure> {
    if status.is_success() {
        Ok(())
    } else {
        Err(DownloadFailure::Download)
    }
}

fn redirect_policy() -> redirect::Policy {
    redirect::Policy::custom(|attempt| {
        if validate_redirect_attempt(attempt.previous().len(), attempt.url()).is_err() {
            return attempt.error("NexusOps updater redirect rejected");
        }
        attempt.follow()
    })
}

fn validate_redirect_attempt(previous: usize, url: &Url) -> Result<(), DownloadFailure> {
    if previous > MAX_REDIRECTS {
        return Err(DownloadFailure::Download);
    }
    validate_redirect_url(url)
}

fn validate_common_url(url: &Url) -> Result<&str, DownloadFailure> {
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.port_or_known_default() != Some(443)
    {
        return Err(DownloadFailure::Download);
    }
    url.host_str().ok_or(DownloadFailure::Download)
}

fn validate_initial_url(url: &Url) -> Result<(), DownloadFailure> {
    let host = validate_common_url(url)?;
    if host != INITIAL_HOST
        || !url.path().starts_with(INITIAL_PATH_PREFIX)
        || url.path().len() <= INITIAL_PATH_PREFIX.len()
    {
        return Err(DownloadFailure::Download);
    }
    Ok(())
}

fn validate_redirect_url(url: &Url) -> Result<(), DownloadFailure> {
    let host = validate_common_url(url)?;
    if REDIRECT_HOSTS.contains(&host) {
        Ok(())
    } else {
        Err(DownloadFailure::Download)
    }
}

fn validate_version(version: &str) -> Result<(), DownloadFailure> {
    if version.is_empty()
        || version.len() > MAX_VERSION_BYTES
        || version.chars().any(char::is_control)
    {
        Err(DownloadFailure::Verification)
    } else {
        Ok(())
    }
}

fn decode_public_key(encoded: &str) -> Result<PublicKey, DownloadFailure> {
    if encoded.is_empty() || encoded.len() > MAX_SIGNATURE_TEXT_BYTES {
        return Err(DownloadFailure::Verification);
    }
    let decoded = STANDARD
        .decode(encoded)
        .map_err(|_| DownloadFailure::Verification)?;
    let text = std::str::from_utf8(&decoded).map_err(|_| DownloadFailure::Verification)?;
    PublicKey::decode(text).map_err(|_| DownloadFailure::Verification)
}

fn decode_signature(encoded: &str) -> Result<Signature, DownloadFailure> {
    if encoded.is_empty() || encoded.len() > MAX_SIGNATURE_TEXT_BYTES {
        return Err(DownloadFailure::Verification);
    }
    let decoded = STANDARD
        .decode(encoded)
        .map_err(|_| DownloadFailure::Verification)?;
    if decoded.len() > MAX_SIGNATURE_TEXT_BYTES {
        return Err(DownloadFailure::Verification);
    }
    let text = std::str::from_utf8(&decoded).map_err(|_| DownloadFailure::Verification)?;
    Signature::decode(text).map_err(|_| DownloadFailure::Verification)
}

fn verify_signed_version(
    trusted_comment: &str,
    announced_version: &str,
) -> Result<(), DownloadFailure> {
    let mut signed_version = None;
    for field in trusted_comment.split('\t') {
        if let Some(value) = field.strip_prefix("version:")
            && (value.is_empty() || signed_version.replace(value).is_some())
        {
            return Err(DownloadFailure::Verification);
        }
    }
    let signed_version = signed_version.ok_or(DownloadFailure::Verification)?;
    let signed_semver = Version::parse(signed_version.strip_prefix('v').unwrap_or(signed_version));
    let announced_semver = Version::parse(
        announced_version
            .strip_prefix('v')
            .unwrap_or(announced_version),
    );
    let matches = match (signed_semver, announced_semver) {
        (Ok(signed), Ok(announced)) => signed == announced,
        _ => signed_version == announced_version,
    };
    if matches {
        Ok(())
    } else {
        Err(DownloadFailure::Verification)
    }
}

fn checked_next_size(current: usize, chunk: usize, limit: usize) -> Result<usize, DownloadFailure> {
    let next = current
        .checked_add(chunk)
        .ok_or(DownloadFailure::ResourceLimit)?;
    if next > limit {
        Err(DownloadFailure::ResourceLimit)
    } else {
        Ok(next)
    }
}

struct BoundedCollector<'a> {
    verifier: StreamVerifier<'a>,
    bytes: Vec<u8>,
    limit: usize,
}

impl<'a> BoundedCollector<'a> {
    fn new(
        verifier: StreamVerifier<'a>,
        content_length: Option<u64>,
        limit: usize,
    ) -> Result<Self, DownloadFailure> {
        let capacity = match content_length {
            Some(length) => {
                let length = usize::try_from(length).map_err(|_| DownloadFailure::ResourceLimit)?;
                if length > limit {
                    return Err(DownloadFailure::ResourceLimit);
                }
                length
            }
            None => 0,
        };
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(capacity)
            .map_err(|_| DownloadFailure::ResourceLimit)?;
        Ok(Self {
            verifier,
            bytes,
            limit,
        })
    }

    fn accept(&mut self, chunk: &[u8]) -> Result<(), DownloadFailure> {
        checked_next_size(self.bytes.len(), chunk.len(), self.limit)?;
        self.bytes
            .try_reserve(chunk.len())
            .map_err(|_| DownloadFailure::ResourceLimit)?;
        self.verifier.update(chunk);
        self.bytes.extend_from_slice(chunk);
        Ok(())
    }

    fn finish(mut self, signature: &'a Signature) -> Result<(Vec<u8>, &'a str), DownloadFailure> {
        self.verifier
            .finalize()
            .map_err(|_| DownloadFailure::Verification)?;
        Ok((self.bytes, signature.trusted_comment()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_ARTIFACT: &[u8] = b"NexusOps Goal 04E disposable signed artifact fixture\n";
    const TEST_PUBLIC_KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDhENjVDOEYzMDA3ODEwODEKUldTQkVIZ0E4OGhsallzdUo2cnFCaUprM3VKYWRRUUpKanJwc0hGL0gvZk1BZGJtYXpkZE1wUXcK";
    const TEST_SIGNATURE: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVTQkVIZ0E4OGhsalpUeHVPRWhRbkdrYTVzTFY0WGVoVlpBOGl4ZlQ5dTl6MkRnV1k4ditKME1jUEJ6dUpMVXdkUmpMbFNhT1NUSjNLL1cyelZmWnhVK0RQSUVZTjF0d3djPQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzkwNzkxMDA3CWZpbGU6YXJ0aWZhY3QuYmluCXZlcnNpb246MS4yLjMKaG5yRDBFVER4c05TbUFwcnYrckJtL2VvSTdiUFFCbktjYVFucmk3TCt4eVN0TW0zTDZKNThlTDVkQU5xcFFnVHhudURyWitwZ09kaE1SV2xPT05hRGc9PQo=";

    fn url(value: &str) -> Url {
        Url::parse(value).unwrap()
    }

    fn verify_fixture(
        bytes: &[u8],
        key: &str,
        signature: &str,
        announced: &str,
    ) -> Result<Vec<u8>, DownloadFailure> {
        let public_key = decode_public_key(key)?;
        let signature = decode_signature(signature)?;
        let verifier = public_key
            .verify_stream(&signature)
            .map_err(|_| DownloadFailure::Verification)?;
        let mut collector = BoundedCollector::new(verifier, None, MAX_ARTIFACT_BYTES)?;
        collector.accept(bytes)?;
        let (bytes, comment) = collector.finish(&signature)?;
        verify_signed_version(comment, announced)?;
        Ok(bytes)
    }

    #[test]
    fn tauri_style_prehashed_signature_streams_and_binds_version() {
        assert_eq!(
            verify_fixture(TEST_ARTIFACT, TEST_PUBLIC_KEY, TEST_SIGNATURE, "1.2.3").unwrap(),
            TEST_ARTIFACT
        );
        assert!(verify_fixture(TEST_ARTIFACT, TEST_PUBLIC_KEY, TEST_SIGNATURE, "v1.2.3").is_ok());
    }

    #[test]
    fn changed_truncated_and_wrong_key_artifacts_fail() {
        let mut changed = TEST_ARTIFACT.to_vec();
        changed[0] ^= 1;
        assert_eq!(
            verify_fixture(&changed, TEST_PUBLIC_KEY, TEST_SIGNATURE, "1.2.3"),
            Err(DownloadFailure::Verification)
        );
        assert_eq!(
            verify_fixture(
                &TEST_ARTIFACT[..TEST_ARTIFACT.len() - 1],
                TEST_PUBLIC_KEY,
                TEST_SIGNATURE,
                "1.2.3"
            ),
            Err(DownloadFailure::Verification)
        );
        let mut wrong_key = STANDARD.decode(TEST_PUBLIC_KEY).unwrap();
        let last = wrong_key.len() - 2;
        wrong_key[last] ^= 1;
        let wrong_key = STANDARD.encode(wrong_key);
        assert_eq!(
            verify_fixture(TEST_ARTIFACT, &wrong_key, TEST_SIGNATURE, "1.2.3"),
            Err(DownloadFailure::Verification)
        );
    }

    #[test]
    fn malformed_and_legacy_signatures_fail_closed() {
        assert!(decode_signature("not base64").is_err());
        let text = String::from_utf8(STANDARD.decode(TEST_SIGNATURE).unwrap()).unwrap();
        let mut lines = text.lines().map(str::to_owned).collect::<Vec<_>>();
        let mut signature_bytes = STANDARD.decode(&lines[1]).unwrap();
        signature_bytes[..2].copy_from_slice(b"Ed");
        lines[1] = STANDARD.encode(signature_bytes);
        let legacy = STANDARD.encode(format!("{}\n", lines.join("\n")));
        let public_key = decode_public_key(TEST_PUBLIC_KEY).unwrap();
        let signature = decode_signature(&legacy).unwrap();
        assert!(public_key.verify_stream(&signature).is_err());
    }

    #[test]
    fn signed_version_is_required_unique_and_exact_when_not_semver() {
        assert!(verify_signed_version("timestamp:1\tfile:a", "1.2.3").is_err());
        assert!(verify_signed_version("version:1.2.3\tversion:1.2.3", "1.2.3").is_err());
        assert!(verify_signed_version("version:1.2.4", "1.2.3").is_err());
        assert!(verify_signed_version("version:v1.2.3", "1.2.3").is_ok());
        assert!(verify_signed_version("version:vv1.2.3", "1.2.3").is_err());
        assert!(verify_signed_version("version:release-one", "release-one").is_ok());
        assert!(verify_signed_version("version:release-one", "release-two").is_err());
    }

    #[test]
    fn initial_and_redirect_url_policy_is_fixed() {
        assert!(
            validate_initial_url(&url(
                "https://github.com/unrealbg/NexusOps/releases/download/v1/NexusOps.exe"
            ))
            .is_ok()
        );
        assert!(
            validate_initial_url(&url(
                "http://github.com/unrealbg/NexusOps/releases/download/v1/a"
            ))
            .is_err()
        );
        assert!(
            validate_initial_url(&url(
                "https://example.com/unrealbg/NexusOps/releases/download/v1/a"
            ))
            .is_err()
        );
        assert!(
            validate_initial_url(&url("https://github.com/other/repo/releases/download/v1/a"))
                .is_err()
        );
        assert!(
            validate_initial_url(&url(
                "https://user@github.com/unrealbg/NexusOps/releases/download/v1/a"
            ))
            .is_err()
        );
        assert!(
            validate_initial_url(&url(
                "https://github.com/unrealbg/NexusOps/releases/download/v1/a#fragment"
            ))
            .is_err()
        );
        assert!(
            validate_redirect_url(&url("https://release-assets.githubusercontent.com/a")).is_ok()
        );
        assert!(validate_redirect_url(&url("https://objects.githubusercontent.com/a")).is_ok());
        assert!(
            validate_redirect_url(&url("http://release-assets.githubusercontent.com/a")).is_err()
        );
        assert!(validate_redirect_url(&url("https://example.com/a")).is_err());
        assert!(validate_redirect_attempt(MAX_REDIRECTS, &url("https://github.com/a")).is_ok());
        assert!(
            validate_redirect_attempt(MAX_REDIRECTS + 1, &url("https://github.com/a")).is_err()
        );
    }

    #[test]
    fn content_length_and_streaming_ceiling_fail_before_append() {
        let key = decode_public_key(TEST_PUBLIC_KEY).unwrap();
        let signature = decode_signature(TEST_SIGNATURE).unwrap();
        let verifier = key.verify_stream(&signature).unwrap();
        assert!(matches!(
            BoundedCollector::new(
                verifier,
                Some((MAX_ARTIFACT_BYTES as u64) + 1),
                MAX_ARTIFACT_BYTES
            ),
            Err(DownloadFailure::ResourceLimit)
        ));

        assert_eq!(
            checked_next_size(MAX_ARTIFACT_BYTES, 0, MAX_ARTIFACT_BYTES),
            Ok(MAX_ARTIFACT_BYTES)
        );
        assert_eq!(
            checked_next_size(MAX_ARTIFACT_BYTES, 1, MAX_ARTIFACT_BYTES),
            Err(DownloadFailure::ResourceLimit)
        );
        assert_eq!(
            checked_next_size(usize::MAX, 1, usize::MAX),
            Err(DownloadFailure::ResourceLimit)
        );
    }

    #[test]
    fn missing_and_dishonest_content_length_still_obey_stream_limit() {
        let key = decode_public_key(TEST_PUBLIC_KEY).unwrap();
        let signature = decode_signature(TEST_SIGNATURE).unwrap();
        let verifier = key.verify_stream(&signature).unwrap();
        let mut missing = BoundedCollector::new(verifier, None, 4).unwrap();
        missing.accept(b"1234").unwrap();
        assert_eq!(missing.accept(b"5"), Err(DownloadFailure::ResourceLimit));
        assert_eq!(missing.bytes.len(), 4);

        let verifier = key.verify_stream(&signature).unwrap();
        let mut dishonest = BoundedCollector::new(verifier, Some(1), 4).unwrap();
        dishonest.accept(b"1234").unwrap();
        assert_eq!(dishonest.accept(b"5"), Err(DownloadFailure::ResourceLimit));
        assert_eq!(dishonest.bytes.len(), 4);
    }

    #[test]
    fn non_success_status_is_rejected() {
        assert!(validate_status(StatusCode::OK).is_ok());
        assert_eq!(
            validate_status(StatusCode::NOT_FOUND),
            Err(DownloadFailure::Download)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn total_timeout_is_fail_closed() {
        let task = tokio::spawn(with_total_timeout(Duration::from_secs(2), async {
            std::future::pending::<Result<Vec<u8>, DownloadFailure>>().await
        }));
        tokio::time::advance(Duration::from_secs(3)).await;
        assert_eq!(task.await.unwrap(), Err(DownloadFailure::Timeout));
    }

    #[test]
    fn fixed_transport_and_resource_constants_do_not_drift() {
        assert_eq!(MAX_ARTIFACT_BYTES, 134_217_728);
        assert_eq!(CONNECT_TIMEOUT, Duration::from_secs(10));
        assert_eq!(DOWNLOAD_TIMEOUT, Duration::from_secs(300));
        assert_eq!(
            REDIRECT_HOSTS,
            [
                "github.com",
                "release-assets.githubusercontent.com",
                "objects.githubusercontent.com"
            ]
        );
    }
}
