import { useEffect, useState } from 'react';
import type { UpdateOperationSnapshot } from '@nexusops/protocol';
import { Button } from '@nexusops/ui';
import { applicationError, updateApi } from '../api/client';

type PendingAction = 'hydrate' | 'check' | 'download' | 'install' | null;

export function UpdateCheck() {
  const [snapshot, setSnapshot] = useState<UpdateOperationSnapshot | null>(null);
  const [pending, setPending] = useState<PendingAction>('hydrate');
  const [error, setError] = useState<string | null>(null);
  const [confirmInstall, setConfirmInstall] = useState(false);
  const [sealedAfterInstallFailure, setSealedAfterInstallFailure] = useState(false);

  useEffect(() => {
    let current = true;
    void updateApi.state().then(
      (next) => {
        if (current) {
          setSnapshot(next);
          setPending(null);
        }
      },
      (reason: unknown) => {
        if (current) {
          setError(applicationError(reason).message);
          setPending(null);
        }
      },
    );
    return () => {
      current = false;
    };
  }, []);

  async function check() {
    setError(null);
    setConfirmInstall(false);
    setSnapshot(null);
    setPending('check');
    try {
      setSnapshot(await updateApi.check());
    } catch (reason) {
      setError(applicationError(reason).message);
    } finally {
      setPending(null);
    }
  }

  async function download() {
    if (snapshot?.phase !== 'updateAnnounced' || !snapshot.announcementId) return;
    const announcementId = snapshot.announcementId;
    setError(null);
    setSnapshot({ ...snapshot, phase: 'downloading', announcementId: null });
    setPending('download');
    try {
      setSnapshot(await updateApi.download(announcementId));
    } catch (reason) {
      // Native authority is consumed before network access and invalidated on every failure.
      setSnapshot(null);
      setError(applicationError(reason).message);
    } finally {
      setPending(null);
    }
  }

  async function install() {
    if (
      snapshot?.phase !== 'verified' ||
      !snapshot.installationSupported ||
      !snapshot.verifiedArtifactId
    ) {
      return;
    }
    const verifiedArtifactId = snapshot.verifiedArtifactId;
    setError(null);
    setPending('install');
    try {
      await updateApi.install(verifiedArtifactId);
    } catch (reason) {
      const failure = applicationError(reason);
      setSealedAfterInstallFailure(true);
      setConfirmInstall(false);
      setError(
        `${failure.message} NexusOps is sealed for safety; close and restart it before continuing.`,
      );
    } finally {
      setPending(null);
    }
  }

  const busy = pending !== null;
  let result: string | null = null;
  if (snapshot?.phase === 'upToDate') {
    result = `NexusOps ${snapshot.currentVersion} is up to date.`;
  } else if (snapshot?.phase === 'updateAnnounced' && snapshot.availableVersion) {
    result = `Update ${snapshot.availableVersion} is announced.`;
  } else if (
    (snapshot?.phase === 'downloading' || snapshot?.phase === 'verifying') &&
    snapshot.availableVersion
  ) {
    result = 'Downloading and verifying…';
  } else if (snapshot?.phase === 'verified' && snapshot.availableVersion) {
    result = `Update ${snapshot.availableVersion} was downloaded and verified against the NexusOps updater key; not installed.`;
  } else if (snapshot?.phase === 'installing' && snapshot.availableVersion) {
    result = 'Preparing the verified installer…';
  }

  const canDownload =
    !busy && snapshot?.phase === 'updateAnnounced' && snapshot.announcementId !== null;
  const canInstall =
    !busy &&
    !sealedAfterInstallFailure &&
    snapshot?.phase === 'verified' &&
    snapshot.installationSupported &&
    snapshot.verifiedArtifactId !== null;

  return (
    <div className="update-check">
      {result && (
        <span className="update-check-status" role="status">
          {result}
          {snapshot?.phase === 'updateAnnounced' && (
            <span className="update-check-detail">
              Download and verification require a separate explicit action.
            </span>
          )}
          {snapshot?.phase === 'verified' && (
            <span className="update-check-detail">
              {snapshot.installationSupported
                ? 'Installation requires a separate confirmation. NexusOps will not reopen automatically.'
                : 'In-app installation is not enabled on this platform.'}
            </span>
          )}
        </span>
      )}
      {error && (
        <span className="update-check-status update-check-status--error" role="alert">
          {error}
        </span>
      )}
      {canDownload && (
        <Button
          className="update-check-button"
          variant="ghost"
          type="button"
          onClick={() => {
            void download();
          }}
        >
          Download and verify
        </Button>
      )}
      {canInstall && !confirmInstall && (
        <Button
          className="update-check-button"
          variant="ghost"
          type="button"
          onClick={() => setConfirmInstall(true)}
        >
          Install update
        </Button>
      )}
      {canInstall && confirmInstall && (
        <div
          className="update-install-confirmation"
          role="group"
          aria-label="Confirm update installation"
        >
          <span className="update-check-detail">
            NexusOps will close active SSH and terminal sessions, safely stop active transfers,
            and launch the verified Windows installer. NexusOps will not reopen automatically;
            reopen it manually after installation finishes.
          </span>
          <Button
            className="update-check-button"
            variant="ghost"
            type="button"
            onClick={() => setConfirmInstall(false)}
          >
            Cancel
          </Button>
          <Button
            className="update-check-button"
            variant="ghost"
            type="button"
            onClick={() => {
              void install();
            }}
          >
            Install and close NexusOps
          </Button>
        </div>
      )}
      <Button
        className="update-check-button"
        variant="ghost"
        type="button"
        disabled={busy || sealedAfterInstallFailure}
        onClick={() => {
          void check();
        }}
      >
        {pending === 'check'
          ? 'Checking…'
          : pending === 'download'
            ? 'Downloading…'
            : pending === 'install'
              ? 'Preparing installer…'
              : 'Check for updates'}
      </Button>
    </div>
  );
}
