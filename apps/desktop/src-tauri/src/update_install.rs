use crate::{
    lifecycle::LogGuardService, local_access::LocalAccessService, updates::InstallAuthority,
};
use nexus_core::Application;
use nexus_model::{AppError, ErrorCode};
use std::future::Future;
use tauri::{AppHandle, Manager};
use tauri_plugin_updater::Update;

const INSTALL_UNAVAILABLE: &str = "In-app update installation is not available on this platform.";
const INSTALL_FAILED: &str =
    "The verified installer could not be launched. Close and restart NexusOps before continuing.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InstallerIdentity {
    pub version: String,
    pub target: String,
    pub artifact_basename: Option<String>,
}

pub(crate) trait InstallerContext: Send {
    fn identity(&self) -> InstallerIdentity;
    fn invoke(self: Box<Self>, bytes: Vec<u8>) -> Result<(), AppError>;
}

/// Non-Clone wrapper around the exact Update returned by the successful check.
pub(crate) struct RetainedTauriUpdate(Update);

impl RetainedTauriUpdate {
    pub(crate) fn new(update: Update) -> Self {
        Self(update)
    }
}

impl InstallerContext for RetainedTauriUpdate {
    fn identity(&self) -> InstallerIdentity {
        InstallerIdentity {
            version: self.0.version.clone(),
            target: self.0.target.clone(),
            artifact_basename: self
                .0
                .download_url
                .path_segments()
                .and_then(|mut segments| segments.next_back())
                .filter(|segment| !segment.is_empty())
                .map(ToOwned::to_owned),
        }
    }

    fn invoke(self: Box<Self>, bytes: Vec<u8>) -> Result<(), AppError> {
        #[cfg(all(windows, any(target_arch = "x86_64", target_arch = "aarch64")))]
        {
            self.0
                .install(bytes)
                .map_err(|_| AppError::new(ErrorCode::UpdateInstall, INSTALL_FAILED))
        }
        #[cfg(not(all(windows, any(target_arch = "x86_64", target_arch = "aarch64"))))]
        {
            let _ = (self, bytes);
            Err(AppError::new(ErrorCode::UpdateInstall, INSTALL_UNAVAILABLE))
        }
    }
}

pub(crate) const fn installation_supported() -> bool {
    cfg!(all(
        windows,
        any(target_arch = "x86_64", target_arch = "aarch64")
    ))
}

pub(crate) fn validate_installer_identity(identity: &InstallerIdentity) -> Result<(), AppError> {
    if !installation_supported() {
        return Err(AppError::new(ErrorCode::UpdateInstall, INSTALL_UNAVAILABLE));
    }
    if identity.target != "windows" {
        return Err(AppError::new(ErrorCode::UpdateVerification, INSTALL_FAILED));
    }
    #[cfg(target_arch = "x86_64")]
    let architecture = "x64";
    #[cfg(target_arch = "aarch64")]
    let architecture = "arm64";
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    let architecture = "unsupported";
    let expected = format!("NexusOps_{}_{architecture}-setup.exe", identity.version);
    if identity.artifact_basename.as_deref() != Some(expected.as_str()) {
        return Err(AppError::new(ErrorCode::UpdateVerification, INSTALL_FAILED));
    }
    Ok(())
}

trait InstallCleanupSteps {
    fn shutdown_application(&self) -> impl Future<Output = Result<(), AppError>> + Send;
    fn revoke_local_grants(&self) -> Result<(), AppError>;
    fn finalize_logging(&self) -> Result<(), AppError>;
    fn invoke_installer(
        &self,
        authority: InstallAuthority,
    ) -> impl Future<Output = Result<(), AppError>> + Send;
}

struct TauriInstallCleanup {
    app: AppHandle,
}

impl InstallCleanupSteps for TauriInstallCleanup {
    async fn shutdown_application(&self) -> Result<(), AppError> {
        self.app.state::<Application>().shutdown().await
    }

    fn revoke_local_grants(&self) -> Result<(), AppError> {
        self.app.state::<LocalAccessService>().revoke_all()
    }

    fn finalize_logging(&self) -> Result<(), AppError> {
        self.app.state::<LogGuardService>().finalize()
    }

    async fn invoke_installer(&self, authority: InstallAuthority) -> Result<(), AppError> {
        tauri::async_runtime::spawn_blocking(move || authority.invoke())
            .await
            .map_err(|_| AppError::new(ErrorCode::UpdateInstall, INSTALL_FAILED))?
    }
}

async fn run_after_consumption(
    steps: &impl InstallCleanupSteps,
    authority: InstallAuthority,
) -> Result<(), AppError> {
    steps.shutdown_application().await?;
    steps.revoke_local_grants()?;
    steps.finalize_logging()?;
    steps.invoke_installer(authority).await
}

pub(crate) async fn finish_install(
    app: AppHandle,
    authority: InstallAuthority,
) -> Result<(), AppError> {
    run_after_consumption(&TauriInstallCleanup { app }, authority).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    struct FakeContext {
        calls: Arc<Mutex<Vec<&'static str>>>,
        expected: Vec<u8>,
    }

    impl InstallerContext for FakeContext {
        fn identity(&self) -> InstallerIdentity {
            InstallerIdentity {
                version: "0.2.0".into(),
                target: "windows".into(),
                artifact_basename: Some("NexusOps_0.2.0_x64-setup.exe".into()),
            }
        }

        fn invoke(self: Box<Self>, bytes: Vec<u8>) -> Result<(), AppError> {
            assert_eq!(bytes, self.expected);
            self.calls.lock().unwrap().push("installer");
            Ok(())
        }
    }

    struct RecordedSteps {
        calls: Arc<Mutex<Vec<&'static str>>>,
        fail_at: Option<&'static str>,
    }

    impl RecordedSteps {
        fn step(&self, name: &'static str) -> Result<(), AppError> {
            self.calls.lock().unwrap().push(name);
            if self.fail_at == Some(name) {
                Err(AppError::new(ErrorCode::UpdateInstall, "safe test failure"))
            } else {
                Ok(())
            }
        }
    }

    impl InstallCleanupSteps for RecordedSteps {
        async fn shutdown_application(&self) -> Result<(), AppError> {
            self.step("application_shutdown")
        }
        fn revoke_local_grants(&self) -> Result<(), AppError> {
            self.step("local_grant_revocation")
        }
        fn finalize_logging(&self) -> Result<(), AppError> {
            self.step("logging_finalize")
        }
        async fn invoke_installer(&self, authority: InstallAuthority) -> Result<(), AppError> {
            self.step("installer_boundary")?;
            authority.invoke()
        }
    }

    #[tokio::test]
    async fn cleanup_order_precedes_exact_byte_installer_invocation() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let authority = InstallAuthority::new(
            Box::new(FakeContext {
                calls: Arc::clone(&calls),
                expected: vec![1, 2, 3],
            }),
            vec![1, 2, 3],
        );
        run_after_consumption(
            &RecordedSteps {
                calls: Arc::clone(&calls),
                fail_at: None,
            },
            authority,
        )
        .await
        .unwrap();
        assert_eq!(
            *calls.lock().unwrap(),
            [
                "application_shutdown",
                "local_grant_revocation",
                "logging_finalize",
                "installer_boundary",
                "installer"
            ]
        );
    }

    #[tokio::test]
    async fn installer_is_not_invoked_when_any_cleanup_step_fails() {
        for failure in [
            "application_shutdown",
            "local_grant_revocation",
            "logging_finalize",
        ] {
            let calls = Arc::new(Mutex::new(Vec::new()));
            let authority = InstallAuthority::new(
                Box::new(FakeContext {
                    calls: Arc::clone(&calls),
                    expected: vec![7],
                }),
                vec![7],
            );
            assert!(
                run_after_consumption(
                    &RecordedSteps {
                        calls: Arc::clone(&calls),
                        fail_at: Some(failure),
                    },
                    authority,
                )
                .await
                .is_err()
            );
            assert!(!calls.lock().unwrap().contains(&"installer"));
        }
    }
}
