import { useRef, useState, type FormEvent } from 'react';
import type { AuthenticationMethod, CredentialInput, Host, HostInput } from '@nexusops/protocol';
import { Button, Modal, Notice } from '@nexusops/ui';
import { applicationError } from '../../api/client';
import { Icon } from '../../components/Icon';
import { validateHost } from './validation';

export function HostForm({
  host,
  onSave,
  onClose,
}: {
  host: Host | null;
  onSave: (input: HostInput, credential: CredentialInput | null) => Promise<void>;
  onClose: () => void;
}) {
  const [displayName, setDisplayName] = useState(host?.displayName ?? '');
  const [hostname, setHostname] = useState(host?.connection.hostname ?? '');
  const [port, setPort] = useState(String(host?.connection.port ?? 22));
  const [username, setUsername] = useState(host?.connection.username ?? '');
  const [authentication, setAuthentication] = useState<AuthenticationMethod>(
    host?.connection.authentication ?? 'privateKey',
  );
  const [replaceCredential, setReplaceCredential] = useState(!host);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  // Uncontrolled inputs keep secrets out of React state, Zustand and query caches.
  const passwordRef = useRef<HTMLInputElement>(null);
  const privateKeyRef = useRef<HTMLTextAreaElement>(null);
  const passphraseRef = useRef<HTMLInputElement>(null);
  const needsCredential =
    replaceCredential || !host || authentication !== host.connection.authentication;

  function clearSecrets() {
    if (passwordRef.current) passwordRef.current.value = '';
    if (privateKeyRef.current) privateKeyRef.current.value = '';
    if (passphraseRef.current) passphraseRef.current.value = '';
  }
  function close() {
    if (!saving) {
      clearSecrets();
      onClose();
    }
  }
  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const input: HostInput = {
      id: host?.id ?? null,
      displayName: displayName.trim(),
      connection: {
        hostname: hostname.trim(),
        port: Number(port),
        username: username.trim(),
        authentication,
      },
    };
    const validation = validateHost(input);
    if (validation) {
      setError(validation);
      return;
    }
    const credential: CredentialInput | null = needsCredential
      ? {
          password: authentication === 'password' ? (passwordRef.current?.value ?? '') : null,
          privateKey: authentication === 'privateKey' ? (privateKeyRef.current?.value ?? '') : null,
          passphrase: authentication === 'privateKey' ? passphraseRef.current?.value || null : null,
        }
      : null;
    if (
      credential &&
      !(authentication === 'password' ? credential.password : credential.privateKey)?.trim()
    ) {
      setError(
        authentication === 'password' ? 'Enter the SSH password.' : 'Paste the SSH private key.',
      );
      return;
    }
    setError(null);
    setSaving(true);
    clearSecrets();
    try {
      await onSave(input, credential);
      onClose();
    } catch (failure) {
      setError(applicationError(failure).message);
    } finally {
      setSaving(false);
    }
  }

  return (
    <Modal title={host ? 'Edit host' : 'Add a host'} onClose={close} wide>
      <p className="dialog-description">
        Connect to a Linux server over SSH. No remote agent required.
      </p>
      <form
        onSubmit={(event) => {
          void submit(event);
        }}
      >
        <fieldset disabled={saving} className="form-fields">
          <label className="field">
            Display name
            <input
              data-initial-focus
              value={displayName}
              onChange={(event) => setDisplayName(event.target.value)}
              placeholder="e.g. Production gateway"
              maxLength={100}
              required
            />
          </label>
          <div className="form-row form-row--endpoint">
            <label className="field">
              Hostname or IP
              <input
                value={hostname}
                onChange={(event) => setHostname(event.target.value)}
                placeholder="server.example.com"
                autoCapitalize="none"
                spellCheck={false}
                required
              />
            </label>
            <label className="field">
              SSH port
              <input
                value={port}
                onChange={(event) => setPort(event.target.value)}
                type="number"
                min={1}
                max={65535}
                required
              />
            </label>
          </div>
          <label className="field">
            Username
            <input
              value={username}
              onChange={(event) => setUsername(event.target.value)}
              placeholder="deploy"
              autoCapitalize="none"
              spellCheck={false}
              autoComplete="off"
              required
            />
          </label>
          <div className="form-section">
            <Icon name="key" size={16} />
            <h3>Authentication</h3>
          </div>
          <label className="field">
            Method
            <select
              value={authentication}
              onChange={(event) => {
                clearSecrets();
                setAuthentication(event.target.value as AuthenticationMethod);
              }}
            >
              <option value="privateKey">SSH private key</option>
              <option value="password">Password</option>
            </select>
          </label>
          {host && authentication === host.connection.authentication && (
            <label className="checkbox-field">
              <input
                type="checkbox"
                checked={replaceCredential}
                onChange={(event) => {
                  clearSecrets();
                  setReplaceCredential(event.target.checked);
                }}
              />
              Replace saved credentials
            </label>
          )}
          {needsCredential ? (
            authentication === 'privateKey' ? (
              <>
                <label className="field">
                  Private key
                  <textarea
                    ref={privateKeyRef}
                    rows={5}
                    autoComplete="off"
                    spellCheck={false}
                    autoCapitalize="none"
                    placeholder="-----BEGIN OPENSSH PRIVATE KEY-----"
                    required
                  />
                </label>
                <label className="field">
                  Passphrase <span className="field-optional">Optional</span>
                  <input
                    ref={passphraseRef}
                    type="password"
                    autoComplete="new-password"
                    placeholder="For an encrypted private key"
                  />
                </label>
              </>
            ) : (
              <label className="field">
                Password
                <input ref={passwordRef} type="password" autoComplete="new-password" required />
              </label>
            )
          ) : (
            <p className="field-hint">
              The existing credential stays in your operating system's secure store.
            </p>
          )}
        </fieldset>
        <p className="security-hint">
          <Icon name="security" size={15} />
          <span>
            Credentials are protected by your OS secure store. A new server's identity must be
            verified before connecting.
          </span>
        </p>
        {error && (
          <Notice>
            {error}
            {needsCredential && (
              <span> Re-enter your credential if it was cleared after submission.</span>
            )}
          </Notice>
        )}
        <div className="modal-actions">
          <Button type="button" onClick={close} disabled={saving}>
            Cancel
          </Button>
          <Button type="submit" variant="primary" disabled={saving}>
            {saving ? 'Saving…' : host ? 'Save changes' : 'Add host'}
          </Button>
        </div>
      </form>
    </Modal>
  );
}
