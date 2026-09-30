import { useEffect, useState } from 'react';
import type { UpdateOperationSnapshot } from '@nexusops/protocol';
import { Button } from '@nexusops/ui';
import { applicationError, updateApi } from '../api/client';

type PendingAction = 'hydrate' | 'check' | 'download' | null;

export function UpdateCheck() {
  const [snapshot, setSnapshot] = useState<UpdateOperationSnapshot | null>(null);
  const [pending, setPending] = useState<PendingAction>('hydrate');
  const [error, setError] = useState<string | null>(null);

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
    result = `Update ${snapshot.availableVersion} was downloaded and verified against the NexusOps updater key.`;
  }

  const canDownload =
    !busy && snapshot?.phase === 'updateAnnounced' && snapshot.announcementId !== null;

  return (
    <div className="update-check">
      {result && (
        <span className="update-check-status" role="status">
          {result}
          {snapshot?.phase === 'updateAnnounced' && (
            <span className="update-check-detail">
              Download is explicit. Installation is not enabled in this build.
            </span>
          )}
          {snapshot?.phase === 'verified' && (
            <span className="update-check-detail">
              The update is not installed. Installation and restart are not enabled.
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
      <Button
        className="update-check-button"
        variant="ghost"
        type="button"
        disabled={busy}
        onClick={() => {
          void check();
        }}
      >
        {pending === 'check'
          ? 'Checking…'
          : pending === 'download'
            ? 'Downloading…'
            : 'Check for updates'}
      </Button>
    </div>
  );
}
