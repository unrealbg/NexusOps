import { useState } from 'react';
import type { UpdateCheckSnapshot } from '@nexusops/protocol';
import { Button } from '@nexusops/ui';
import { applicationError, updateApi } from '../api/client';

type CheckState =
  | { kind: 'idle' }
  | { kind: 'checking' }
  | { kind: 'result'; snapshot: UpdateCheckSnapshot }
  | { kind: 'error'; message: string };

export function UpdateCheck() {
  const [state, setState] = useState<CheckState>({ kind: 'idle' });
  const checking = state.kind === 'checking';

  async function check() {
    setState({ kind: 'checking' });
    try {
      setState({ kind: 'result', snapshot: await updateApi.check() });
    } catch (error) {
      setState({ kind: 'error', message: applicationError(error).message });
    }
  }

  let result: string | null = null;
  let announced = false;
  if (state.kind === 'result') {
    const { snapshot } = state;
    if (snapshot.status === 'upToDate') {
      result = `NexusOps ${snapshot.currentVersion} is up to date.`;
    } else if (snapshot.status === 'updateAnnounced' && snapshot.availableVersion) {
      result = `Update ${snapshot.availableVersion} is announced.`;
      announced = true;
    } else {
      result = 'The update service could not be checked right now.';
    }
  }

  return (
    <div className="update-check">
      {result && (
        <span className="update-check-status" role="status">
          {result}
          {announced && (
            <span className="update-check-detail">
              Download and installation are not enabled in this build.
            </span>
          )}
        </span>
      )}
      {state.kind === 'error' && (
        <span className="update-check-status update-check-status--error" role="alert">
          {state.message}
        </span>
      )}
      <Button
        className="update-check-button"
        variant="ghost"
        type="button"
        disabled={checking}
        onClick={() => { void check(); }}
      >
        {checking ? 'Checking…' : 'Check for updates'}
      </Button>
    </div>
  );
}
