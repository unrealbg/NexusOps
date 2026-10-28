import { useCallback, useEffect, useId, useLayoutEffect, useRef, useState } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import type { CredentialInput, Host, HostInput } from '@nexusops/protocol';
import { SecurityWorkspace } from './features/security/SecurityWorkspace';
import { Button, Modal, Notice, Spinner } from '@nexusops/ui';
import { hostApi, applicationError } from './api/client';
import { hostKeys, useHosts, useHostSession } from './api/queries';
import { useSelection } from './state/selection';
import { HostSidebar } from './components/HostSidebar';
import { UpdateCheck } from './components/UpdateCheck';
import { HostForm } from './features/hosts/HostForm';
import { HostList } from './features/hosts/HostList';
import { HostOverview } from './features/overview/HostOverview';
import { TerminalWorkspace } from './features/terminal/TerminalWorkspace';
import { FilesWorkspace } from './features/files/FilesWorkspace';
import { ServicesWorkspace } from './features/services/ServicesWorkspace';
import { LogsWorkspace } from './features/logs/LogsWorkspace';
import { ContainersWorkspace } from './features/containers/ContainersWorkspace';
import { NetworkWorkspace } from './features/network/NetworkWorkspace';
import { CompactMonitorStrip } from './features/overview/CompactMonitorStrip';

type DialogState =
  { type: 'add' } | { type: 'edit'; host: Host } | { type: 'delete'; host: Host } | null;

const FILES_MIN_WIDTH = 280;
const TERMINAL_MIN_WIDTH = 400;
const SPLIT_HANDLE_WIDTH = 8;

// A host's two workspaces always occupy these same parent slots. Modes change
// visibility and presentation, never the owner or parent of a mounted pane.
function SshWorkspace({ host, mode, terminalVisited, filesVisited, onShowOverview, onShowFiles }: {
  host: Host; mode: 'terminal' | 'files' | null; terminalVisited: boolean; filesVisited: boolean;
  onShowOverview: () => void; onShowFiles: () => void;
}) {
  const connection = useHostSession(host.id);
  const [combined, setCombined] = useState(false);
  const [filesRequested, setFilesRequested] = useState(false);
  const [fraction, setFraction] = useState(0.33);
  const [width, setWidth] = useState(0);
  const root = useRef<HTMLDivElement>(null);
  const drag = useRef<{ pointer: number; element: HTMLElement; startX: number; startLeft: number } | null>(null);
  const panelId = useId();
  const narrow = width < FILES_MIN_WIDTH + TERMINAL_MIN_WIDTH + SPLIT_HANDLE_WIDTH;
  const split = mode === 'terminal' && combined && !narrow;
  const filesVisible = mode === 'files' || split;
  const maximum = Math.max(FILES_MIN_WIDTH, width - TERMINAL_MIN_WIDTH - SPLIT_HANDLE_WIDTH);
  const left = Math.min(maximum, Math.max(FILES_MIN_WIDTH, (width - SPLIT_HANDLE_WIDTH) * fraction));

  const stopDrag = useCallback(() => {
    const current = drag.current;
    drag.current = null;
    if (current?.element.hasPointerCapture(current.pointer)) current.element.releasePointerCapture(current.pointer);
  }, []);

  useLayoutEffect(() => {
    const element = root.current;
    if (!element) return;
    const measure = () => setWidth(element.getBoundingClientRect().width);
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    measure();
    return () => observer.disconnect();
  }, []);
  useEffect(() => stopDrag, [split, stopDrag]);

  function resize(next: number) {
    if (!split) return;
    setFraction(Math.min(maximum, Math.max(FILES_MIN_WIDTH, next)) / (width - SPLIT_HANDLE_WIDTH));
  }

  return <div className="ssh-host-root" hidden={mode === null}>
    <div ref={root} className={`ssh-layout ${split ? 'ssh-layout--split' : ''}`}
      style={split ? { gridTemplateColumns: `${left}px ${SPLIT_HANDLE_WIDTH}px minmax(0, 1fr)` } : undefined}>
      <div id={`${panelId}-files`} className="ssh-files-slot" hidden={!filesVisible}>
        {(filesVisited || filesRequested) && <FilesWorkspace host={host} visible={filesVisible}
          presentation={mode === 'files' ? 'full' : 'compact'} onShowFull={onShowFiles} onShowOverview={onShowOverview} />}
      </div>
      <div className="ssh-separator" hidden={!split} role="separator" aria-label="Resize files panel"
        aria-orientation="vertical" aria-controls={`${panelId}-files ${panelId}-terminal`}
        aria-valuemin={FILES_MIN_WIDTH} aria-valuemax={Math.round(maximum)} aria-valuenow={Math.round(left)}
        aria-valuetext={`${Math.round(left)} pixels for files`} tabIndex={split ? 0 : -1}
        onKeyDown={(event) => {
          const next = event.key === 'ArrowLeft' ? left - 16 : event.key === 'ArrowRight' ? left + 16
            : event.key === 'Home' ? FILES_MIN_WIDTH : event.key === 'End' ? maximum : null;
          if (next !== null) { event.preventDefault(); resize(next); }
        }}
        onPointerDown={(event) => {
          if (!split || event.button !== 0) return;
          event.preventDefault();
          event.currentTarget.focus();
          event.currentTarget.setPointerCapture(event.pointerId);
          drag.current = { pointer: event.pointerId, element: event.currentTarget, startX: event.clientX, startLeft: left };
        }}
        onPointerMove={(event) => {
          if (drag.current?.pointer === event.pointerId) resize(drag.current.startLeft + event.clientX - drag.current.startX);
        }}
        onPointerUp={stopDrag} onPointerCancel={stopDrag} onLostPointerCapture={stopDrag} />
      <div id={`${panelId}-terminal`} className="ssh-terminal-slot" hidden={mode !== 'terminal'}>
        {terminalVisited && <TerminalWorkspace host={host} visible={mode === 'terminal'} onShowOverview={onShowOverview}
          combined={combined} onToggleFiles={() => { setFilesRequested(true); setCombined((current) => !current); }} />}
      </div>
    </div>
    {mode === 'terminal' && combined && narrow && <div className="ssh-collapsed-files" role="status">
      Files panel collapsed to keep the terminal usable. <Button onClick={onShowFiles}>Open full Files view</Button>
    </div>}
    <CompactMonitorStrip hostId={host.id} hostSessionId={connection.data?.hostSessionId ?? null}
      connected={connection.data?.state === 'connected'} visible={mode === 'terminal' && combined} />
  </div>;
}

export default function App() {
  const hostsQuery = useHosts();
  const queryClient = useQueryClient();
  const { hostId, select } = useSelection();
  const [dialog, setDialog] = useState<DialogState>(null);
  const [deleteError, setDeleteError] = useState<string | null>(null);
  const [deleting, setDeleting] = useState(false);
  const [section, setSection] = useState<'overview' | 'terminal' | 'files' | 'services' | 'network' | 'security' | 'logs' | 'containers'>('overview');
  const [terminalHosts, setTerminalHosts] = useState<string[]>([]);
  const [fileHosts, setFileHosts] = useState<string[]>([]);
  const hosts = hostsQuery.data ?? [];
  const selectedHost = hosts.find((host) => host.id === hostId) ?? null;

  function visitTerminal(host: Host) {
    setTerminalHosts((current) => (current.includes(host.id) ? current : [...current, host.id]));
  }

  function selectHost(id: string | null) {
    select(id);
    const host = hosts.find((candidate) => candidate.id === id);
    if (host && section === 'terminal') visitTerminal(host);
    if (host && section === 'files') setFileHosts((current) => current.includes(host.id) ? current : [...current, host.id]);
  }

  function selectSection(next: 'overview' | 'terminal' | 'files' | 'services' | 'network' | 'security' | 'logs' | 'containers') {
    setSection(next);
    if (next === 'terminal' && selectedHost) visitTerminal(selectedHost);
    if (next === 'files' && selectedHost) setFileHosts((current) => current.includes(selectedHost.id) ? current : [...current, selectedHost.id]);
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
            {selectedHost && <span>{section === 'terminal' ? 'Terminal' : section === 'files' ? 'Files' : section === 'services' ? 'Services' : section === 'network' ? 'Network' : section === 'security' ? 'Security' : section === 'logs' ? 'Logs' : section === 'containers' ? 'Containers' : 'Overview'}</span>}
          </div>
          <div className="topbar-actions">
            <UpdateCheck />
            <span className="topbar-label">
              <span className="local-dot" />
              Local control plane
            </span>
          </div>
        </div>
        <main
          id="main-content"
          tabIndex={-1}
          className={`main-content ${section === 'terminal' ? 'main-content--terminal' : ''} ${section === 'files' ? 'main-content--files' : ''}`}
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
          {[...new Set([...terminalHosts, ...fileHosts])].map((id) => {
            const host = hosts.find((candidate) => candidate.id === id);
            if (!host) return null;
            return <SshWorkspace key={host.id} host={host}
              mode={selectedHost?.id === id && (section === 'terminal' || section === 'files') ? section : null}
              terminalVisited={terminalHosts.includes(id)} filesVisited={fileHosts.includes(id)}
              onShowOverview={() => setSection('overview')} onShowFiles={() => selectSection('files')} />;
          })}
          {selectedHost && section === 'services' && (
            <ServicesWorkspace key={selectedHost.id} host={selectedHost} />
          )}
          {selectedHost && section === 'logs' && <LogsWorkspace key={selectedHost.id} host={selectedHost} />}
          {selectedHost && section === 'containers' && <ContainersWorkspace key={selectedHost.id} host={selectedHost} />}
          {selectedHost && section === 'network' && (
            <NetworkWorkspace key={selectedHost.id} host={selectedHost} />
          )}
          {selectedHost && section === 'security' && (
            <SecurityWorkspace key={selectedHost.id} host={selectedHost} />
          )}
        </main>
        <footer className="app-footer">
          <span>
            NexusOps <span className="footer-divider">/</span> {section === 'files' ? 'SFTP files' : section === 'services' ? 'System services' : section === 'network' ? 'Network interfaces' : section === 'security' ? 'SSH endpoint trust' : section === 'containers' ? 'Docker containers' : 'Terminal workspace'}
          </span>
          <span>{section === 'security' ? 'Local trust metadata · on demand' : section === 'files' ? 'Agentless · streamed transfers' : section === 'services' || section === 'network' || section === 'containers' || section === 'logs' ? 'Read-only · on demand' : 'Interactive PTY · local scrollback'}</span>
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
