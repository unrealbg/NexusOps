import { useState } from 'react';
import type { AppError, HostKeyChallenge } from '@nexusops/protocol';
import { Button, Modal, Notice } from '@nexusops/ui';
import { Icon } from '../../components/Icon';

function Fingerprint({ challenge }: { challenge: HostKeyChallenge }) {
  return (
    <dl className="trust-details">
      <div>
        <dt>Server</dt>
        <dd>
          {challenge.hostname}:{challenge.port}
        </dd>
      </div>
      <div>
        <dt>Algorithm</dt>
        <dd>{challenge.algorithm}</dd>
      </div>
      <div>
        <dt>SHA-256 fingerprint</dt>
        <dd className="fingerprint">{challenge.fingerprint}</dd>
      </div>
      {challenge.previousFingerprint && (
        <div>
          <dt>Previously trusted fingerprint</dt>
          <dd className="fingerprint">{challenge.previousFingerprint}</dd>
        </div>
      )}
    </dl>
  );
}

export function HostKeyNotice({
  error,
  onTrust,
  busy,
}: {
  error: AppError;
  onTrust: (challenge: HostKeyChallenge) => void;
  busy: boolean;
}) {
  const [showTrust, setShowTrust] = useState(false);
  if (error.code === 'changedHostKey') {
    return (
      <section className="trust-block">
        <Notice>
          <div className="notice-heading">
            <Icon name="shield" />
            <strong>Host key changed. Connection blocked.</strong>
          </div>
          <p>
            This server presented a different key from the one you trusted. Verify the change
            through a separate trusted channel before updating your local trust record.
          </p>
          {error.hostKey && <Fingerprint challenge={error.hostKey} />}
          <p>NexusOps will not accept this changed key from this screen.</p>
        </Notice>
      </section>
    );
  }
  if (error.code !== 'unknownHostKey' || !error.hostKey) return null;
  const challenge = error.hostKey;
  return (
    <>
      <Notice tone="warning">
        <div className="notice-heading">
          <Icon name="shield" />
          <strong>Verify this server's identity</strong>
        </div>
        <p>
          This is your first connection to this endpoint. Review its SSH fingerprint before any
          credentials are sent.
        </p>
        <Button onClick={() => setShowTrust(true)} disabled={busy}>
          Review fingerprint
          <Icon name="arrow" size={15} />
        </Button>
      </Notice>
      {showTrust && (
        <Modal title="Trust this host key?" onClose={() => setShowTrust(false)} wide>
          <p className="dialog-description">
            Compare this fingerprint with a trusted source, such as your server console or
            administrator. Trust applies to this exact hostname and port.
          </p>
          <Fingerprint challenge={challenge} />
          <Notice tone="warning">
            An unverified key could belong to an impersonating server. Continue only when you
            recognize this identity.
          </Notice>
          <div className="modal-actions">
            <Button onClick={() => setShowTrust(false)}>Cancel</Button>
            <Button
              variant="primary"
              disabled={busy}
              onClick={() => {
                onTrust(challenge);
                setShowTrust(false);
              }}
            >
              Trust and connect
            </Button>
          </div>
        </Modal>
      )}
    </>
  );
}
