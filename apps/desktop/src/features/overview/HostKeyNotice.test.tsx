import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { HostKeyNotice } from './HostKeyNotice';

const challenge = {
  hostname: 'host.example.test',
  port: 2222,
  algorithm: 'ssh-ed25519',
  fingerprint: 'SHA256:new-test-fingerprint',
  previousFingerprint: null,
};

describe('host identity review', () => {
  it('requires an explicit review and trust action bound to the exact challenge', async () => {
    const trust = vi.fn();
    render(
      <HostKeyNotice
        error={{ code: 'unknownHostKey', message: 'Unknown host key.', hostKey: challenge }}
        onTrust={trust}
        busy={false}
      />,
    );
    expect(trust).not.toHaveBeenCalled();
    expect(screen.queryByRole('button', { name: 'Trust and connect' })).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Review fingerprint' }));
    expect(screen.getByText('host.example.test:2222')).toBeInTheDocument();
    expect(screen.getByText('ssh-ed25519')).toBeInTheDocument();
    expect(screen.getByText(challenge.fingerprint)).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Trust and connect' }));
    expect(trust).toHaveBeenCalledWith(challenge);
  });
  it('offers no trust override for a changed fingerprint', () => {
    const trust = vi.fn();
    render(
      <HostKeyNotice
        error={{
          code: 'changedHostKey',
          message: 'Changed host key.',
          hostKey: { ...challenge, previousFingerprint: 'SHA256:previous-test-key' },
        }}
        onTrust={trust}
        busy={false}
      />,
    );
    expect(screen.getByRole('alert')).toHaveTextContent('Connection blocked');
    expect(screen.getByText('SHA256:previous-test-key')).toBeInTheDocument();
    expect(screen.queryByRole('button')).not.toBeInTheDocument();
    expect(trust).not.toHaveBeenCalled();
  });
  it('does not trust when the review is dismissed', async () => {
    const trust = vi.fn();
    render(
      <HostKeyNotice
        error={{ code: 'unknownHostKey', message: '', hostKey: challenge }}
        onTrust={trust}
        busy={false}
      />,
    );
    await userEvent.click(screen.getByRole('button', { name: 'Review fingerprint' }));
    await userEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(trust).not.toHaveBeenCalled();
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });
});
