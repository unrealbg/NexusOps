import { useState } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import type { CredentialInput, Host, HostInput } from '@nexusops/protocol';
import { Button, Modal, Notice, Spinner } from '@nexusops/ui';
import { hostApi, applicationError } from './api/client';
import { hostKeys, useHosts } from './api/queries';
import { useSelection } from './state/selection';
import { HostSidebar } from './components/HostSidebar';
import { HostForm } from './features/hosts/HostForm';
import { HostList } from './features/hosts/HostList';
import { HostOverview } from './features/overview/HostOverview';
import { TerminalWorkspace } from './features/terminal/TerminalWorkspace';

type DialogState =
  { type: 'add' } | { type: 'edit'; host: Host } | { type: 'delete'; host: Host } | null;

export default function App() {
  const hostsQuery = useHosts();
  const queryClient = useQueryClient();
  const { hostId, select } = useSelection();
  const [dialog, setDialog] = useState<DialogState>(null);
  const [deleteError, setDeleteError] = useState<string | null>(null);
  const [deleting, setDeleting] = useState(false);
  const [section, setSection] = useState<'overview' | 'terminal'>('overview');
  const [terminalHosts, setTerminalHosts] = useState<string[]>([]);
  const hosts = hostsQuery.data ?? [];
  const selectedHost = hosts.find((host) => host.id === hostId) ?? null;

  function visitTerminal(host: Host) {
    setTerminalHosts((current) => (current.includes(host.id) ? current : [...current, host.id]));
  }

  function selectHost(id: string | null) {
    select(id);
    const host = hosts.find((candidate) => candidate.id === id);
    if (host && section === 'terminal') visitTerminal(host);
  }

  function selectSection(next: 'overview' | 'terminal') {
    setSection(next);
    if (next === 'terminal' && selectedHost) visitTerminal(selectedHost);
  }

  // Do not move this into useMutation: its cache retains mutation variables, including secrets.
  async function saveHost(input: HostInput, credential: CredentialInput | null) {
    const saved = await hostApi.save(input, credential);
    await queryClient.invalidateQueries({ queryKey: hostKeys.all });
    await queryClient.invalidateQueries({ queryKey: hostKeys.session(saved.id) });
    select(saved.id);
    if (section === 'terminal') visitTerminal(saved);
  }
  async function deleteHost(host: Host) {
    setDeleting(true);
    setDeleteError(null);
    try {
      await hostApi.delete(host.id);
      queryClient.removeQueries({ queryKey: hostKeys.session(host.id) });
      await queryClient.invalidateQueries({ queryKey: hostKeys.all });
      if (hostId === host.id) select(null);
      setDialog(null);
    } catch (error) {
      setDeleteError(applicationError(error).message);
    } finally {
      setDeleting(false);
    }
  }

  return (
    <div className="app-shell">
      <a className="skip-link" href="#main-content">
        Skip to main content
      </a>
      <HostSidebar
        hosts={hosts}
        selectedId={selectedHost?.id ?? null}
        onSelect={selectHost}
        onAdd={() => setDialog({ type: 'add' })}
        activeSection={section}
        onSection={selectSection}
      />
      <div className="main-shell">
        <div className="topbar">
          <div className="breadcrumbs">
            <button onClick={() => selectHost(null)}>Workspace</button>
            <span>/</span>
            <span>{selectedHost?.displayName ?? 'Overview'}</span>
            {selectedHost && <span>{section === 'terminal' ? 'Terminal' : 'Overview'}</span>}
          </div>
          <span className="topbar-label">
            <span className="local-dot" />
            Local control plane
          </span>
        </div>
        <main
          id="main-content"
          tabIndex={-1}
          className={`main-content ${section === 'terminal' ? 'main-content--terminal' : ''}`}
        >
          {hostsQuery.isPending ? (
            <div className="state-panel">
              <Spinner label="Loading your workspace…" />
            </div>
          ) : hostsQuery.isError ? (
            <>
              <header className="page-heading">
                <div>
                  <div className="eyebrow">LOCAL WORKSPACE</div>
                  <h1>Your infrastructure</h1>
                </div>
              </header>
              <Notice>
                {applicationError(hostsQuery.error).message}
                <div className="notice-action">
                  <Button
                    onClick={() => {
                      void hostsQuery.refetch();
                    }}
                  >
                    Try again
                  </Button>
                </div>
              </Notice>
            </>
          ) : selectedHost && section === 'overview' ? (
            <HostOverview
              key={selectedHost.id}
              host={selectedHost}
              onEdit={() => setDialog({ type: 'edit', host: selectedHost })}
              onDelete={() => {
                setDeleteError(null);
                setDialog({ type: 'delete', host: selectedHost });
              }}
            />
          ) : !selectedHost ? (
            <HostList hosts={hosts} onSelect={selectHost} onAdd={() => setDialog({ type: 'add' })} />
          ) : null}
          {terminalHosts.map((terminalHostId) => {
            const terminalHost = hosts.find((host) => host.id === terminalHostId);
            if (!terminalHost) return null;
            const visible = section === 'terminal' && selectedHost?.id === terminalHost.id;
            return (
              <div key={terminalHost.id} hidden={!visible} className="terminal-host-root">
                <TerminalWorkspace
                  host={terminalHost}
                  visible={visible}
                  onShowOverview={() => setSection('overview')}
                />
              </div>
            );
          })}
        </main>
        <footer className="app-footer">
          <span>
            NexusOps <span className="footer-divider">/</span> Terminal workspace
          </span>
          <span>Interactive PTY · local scrollback</span>
        </footer>
      </div>
      {dialog && dialog.type !== 'delete' && (
        <HostForm
          key={dialog.type === 'edit' ? dialog.host.id : 'new'}
          host={dialog.type === 'edit' ? dialog.host : null}
          onSave={saveHost}
          onClose={() => setDialog(null)}
        />
      )}
      {dialog?.type === 'delete' && (
        <Modal
          title="Delete this host?"
          onClose={() => {
            if (!deleting) setDialog(null);
          }}
        >
          <p className="dialog-description">
            <strong>{dialog.host.displayName}</strong> and its saved credentials will be removed
            from this workspace. This does not make any changes to the remote server.
          </p>
          {deleteError && <Notice>{deleteError}</Notice>}
          <div className="modal-actions">
            <Button disabled={deleting} onClick={() => setDialog(null)}>
              Cancel
            </Button>
            <Button
              variant="danger"
              disabled={deleting}
              onClick={() => {
                void deleteHost(dialog.host);
              }}
            >
              {deleting ? 'Deleting…' : 'Delete host'}
            </Button>
          </div>
        </Modal>
      )}
    </div>
  );
}
