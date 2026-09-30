use nexus_model::{AppError, ErrorCode, UpdateCheckSnapshot, UpdateCheckStatus};
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use tauri::AppHandle;
use tauri_plugin_updater::UpdaterExt;

const CHECK_TIMEOUT: Duration = Duration::from_secs(15);
const CHECK_UNAVAILABLE: &str = "The update service could not be checked right now.";
const CHECK_IN_PROGRESS: &str = "An update check is already in progress.";

/// Application-global admission and fixed native updater policy.
#[derive(Default)]
pub struct UpdateCheckService {
    in_flight: AtomicBool,
}

struct CheckPermit<'a>(&'a AtomicBool);

impl Drop for CheckPermit<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

impl UpdateCheckService {
    fn admit(&self) -> Result<CheckPermit<'_>, AppError> {
        self.in_flight
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .map_err(|_| AppError::new(ErrorCode::Conflict, CHECK_IN_PROGRESS))?;
        Ok(CheckPermit(&self.in_flight))
    }

    pub async fn check(&self, app: &AppHandle) -> Result<UpdateCheckSnapshot, AppError> {
        let _permit = self.admit()?;
        let current_version = app.package_info().version.to_string();
        let updater = app
            .updater_builder()
            .timeout(CHECK_TIMEOUT)
            .no_proxy()
            .build()
            .map_err(|_| check_unavailable())?;
        let announced = updater.check().await.map_err(|_| check_unavailable())?;
        Ok(project_result(
            current_version,
            announced.map(|update| update.version),
        ))
    }
}

fn check_unavailable() -> AppError {
    // Upstream errors can contain URLs and manifest or transport details.
    AppError::new(ErrorCode::UpdateCheck, CHECK_UNAVAILABLE)
}

fn project_result(
    current_version: String,
    announced_version: Option<String>,
) -> UpdateCheckSnapshot {
    match announced_version {
        Some(version) => UpdateCheckSnapshot {
            current_version,
            status: UpdateCheckStatus::UpdateAnnounced,
            available_version: Some(version),
        },
        None => UpdateCheckSnapshot {
            current_version,
            status: UpdateCheckStatus::UpToDate,
            available_version: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn one_check_is_admitted_globally_and_the_permit_releases_on_drop() {
        let service = Arc::new(UpdateCheckService::default());
        let first = service.admit().expect("first check must be admitted");
        let concurrent = Arc::clone(&service);
        let error = std::thread::spawn(move || {
            concurrent
                .admit()
                .err()
                .expect("a simultaneous check must fail fast")
        })
        .join()
        .expect("admission test thread must finish");
        assert_eq!(error.code, ErrorCode::Conflict);
        assert_eq!(error.message, CHECK_IN_PROGRESS);
        drop(first);
        assert!(service.admit().is_ok());
    }

    #[test]
    fn check_results_only_project_status_and_versions() {
        assert_eq!(
            project_result("0.1.0".into(), None),
            UpdateCheckSnapshot {
                current_version: "0.1.0".into(),
                status: UpdateCheckStatus::UpToDate,
                available_version: None,
            }
        );
        assert_eq!(
            project_result("0.1.0".into(), Some("0.2.0".into())),
            UpdateCheckSnapshot {
                current_version: "0.1.0".into(),
                status: UpdateCheckStatus::UpdateAnnounced,
                available_version: Some("0.2.0".into()),
            }
        );
    }
}
