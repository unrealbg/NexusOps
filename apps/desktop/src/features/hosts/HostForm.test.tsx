import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { HostForm } from './HostForm';
import { host } from '../../test/fixtures';

describe('host credentials', () => {
  it('retains the existing credential on a metadata-only edit', async () => {
    const save = vi.fn().mockResolvedValue(undefined);
    render(<HostForm host={host} onSave={save} onClose={vi.fn()} />);
    expect(screen.queryByLabelText('Password')).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Save changes' }));
    await waitFor(() =>
      expect(save).toHaveBeenCalledWith(
        { id: host.id, displayName: host.displayName, connection: host.connection },
        null,
      ),
    );
  });
  it('clears submitted secret fields immediately, including on secure-store failure', async () => {
    let rejectSave: (reason: unknown) => void = () => undefined;
    const save = vi.fn(
      () =>
        new Promise<void>((_resolve, reject) => {
          rejectSave = reject;
        }),
    );
    render(<HostForm host={host} onSave={save} onClose={vi.fn()} />);
    await userEvent.click(screen.getByRole('checkbox', { name: 'Replace saved credentials' }));
    const password = screen.getByLabelText('Password');
    await userEvent.type(password, 'transient-test-secret');
    await userEvent.click(screen.getByRole('button', { name: 'Save changes' }));
    expect(password).toHaveValue('');
    expect(save).toHaveBeenCalledWith(expect.anything(), {
      password: 'transient-test-secret',
      privateKey: null,
      passphrase: null,
    });
    rejectSave({
      code: 'secureStorage',
      message: 'The OS secure store is unavailable.',
      hostKey: null,
    });
    expect(await screen.findByRole('alert')).toHaveTextContent(
      'The OS secure store is unavailable.',
    );
    expect(password).toHaveValue('');
  });
  it('requires a replacement when changing the authentication method', async () => {
    const save = vi.fn();
    const { container } = render(<HostForm host={host} onSave={save} onClose={vi.fn()} />);
    await userEvent.selectOptions(screen.getByLabelText('Method'), 'privateKey');
    fireEvent.submit(container.querySelector('form')!);
    expect(await screen.findByRole('alert')).toHaveTextContent('Paste the SSH private key.');
    expect(save).not.toHaveBeenCalled();
  });
  it('clears credentials before dismissal', async () => {
    const close = vi.fn();
    render(<HostForm host={host} onSave={vi.fn()} onClose={close} />);
    await userEvent.click(screen.getByRole('checkbox', { name: 'Replace saved credentials' }));
    const password = screen.getByLabelText('Password');
    await userEvent.type(password, 'discarded-secret');
    await userEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(password).toHaveValue('');
    expect(close).toHaveBeenCalledOnce();
  });
});
