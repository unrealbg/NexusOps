import { useEffect, useMemo, useRef, useState } from 'react';
import type {
  ConflictPolicy, FileOperationPlan, Host, RemoteEntry, SftpSessionInfo, TransferJob,
} from '@nexusops/protocol';
import { Button, Modal, Notice, Spinner } from '@nexusops/ui';
import { applicationError, filesApi } from '../../api/client';
import { useHostSession } from '../../api/queries';

type EntryDialog = { type: 'mkdir'; value: string } | { type: 'rename'; entry: RemoteEntry; value: string } | null;

export function FilesWorkspace({ host, visible, onShowOverview }: { host: Host; visible: boolean; onShowOverview: () => void }) {
  const connection = useHostSession(host.id);
  const [sftp, setSftp] = useState<SftpSessionInfo | null>(null);
  const [listing, setListing] = useState<{ path: string; entries: RemoteEntry[]; partial: boolean; entryCap: number } | null>(null);
  const [address, setAddress] = useState('');
  const [history, setHistory] = useState<string[]>([]);
  const [historyIndex, setHistoryIndex] = useState(-1);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [filter, setFilter] = useState('');
  const [showHidden, setShowHidden] = useState(false);
  const [sort, setSort] = useState<'name' | 'size' | 'modified'>('name');
  const [descending, setDescending] = useState(false);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [stale, setStale] = useState(false);
  const [conflictPolicy, setConflictPolicy] = useState<ConflictPolicy>('skip');
  const [plans, setPlans] = useState<FileOperationPlan[]>([]);
  const [dialog, setDialog] = useState<EntryDialog>(null);
  const [properties, setProperties] = useState<RemoteEntry | null>(null);
  const [transfers, setTransfers] = useState<TransferJob[]>([]);
  const requestGeneration = useRef(0);

  const connected = connection.data?.state === 'connected';
  const canMutate = connected && !!sftp && !stale && !loading;

  async function loadDirectory(session: SftpSessionInfo, target: string, mode: 'push' | 'replace' | 'history' = 'push') {
    const generation = ++requestGeneration.current;
    setLoading(true);
    setError(null);
    try {
      const result = await filesApi.list(session, target);
      if (generation !== requestGeneration.current) return;
      setListing({ path: result.path, entries: result.entries, partial: result.partial, entryCap: result.entryCap });
      setAddress(result.path);
      setSelected(new Set());
      setStale(false);
      if (mode === 'push') {
        setHistory((current) => [...current.slice(0, historyIndex + 1), result.path]);
        setHistoryIndex((current) => current + 1);
      } else if (mode === 'replace') {
        setHistory([result.path]);
        setHistoryIndex(0);
      }
    } catch (reason) {
      if (generation === requestGeneration.current) setError(applicationError(reason).message);
    } finally {
      if (generation === requestGeneration.current) setLoading(false);
    }
  }

  useEffect(() => {
    if (!visible) return;
    if (!connected) {
      requestGeneration.current += 1;
      let active = true;
      void Promise.resolve().then(() => {
        if (!active) return;
        setSftp(null);
        if (listing) setStale(true);
      });
      return () => { active = false; };
    }
    let active = true;
    void filesApi.open(host.id).then((session) => {
      if (!active) return;
      setSftp(session);
      void loadDirectory(session, listing?.path ?? session.rootPath, listing ? 'history' : 'replace');
    }).catch((reason) => { if (active) setError(applicationError(reason).message); });
    return () => { active = false; };
    // Reopen after a visible disconnect/reconnect transition; keep location by host.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [host.id, connected, visible]);

  useEffect(() => {
    if (!visible) return;
    let active = true;
    const poll = () => { void filesApi.transfers(host.id).then((value) => { if (active) setTransfers(value); }).catch(() => undefined); };
    poll();
    const timer = window.setInterval(poll, 500);
    return () => { active = false; window.clearInterval(timer); };
  }, [host.id, visible]);

  const entries = useMemo(() => {
    const query = filter.toLocaleLowerCase();
    return [...(listing?.entries ?? [])]
      .filter((entry) => (showHidden || !entry.name.startsWith('.')) && entry.displayName.toLocaleLowerCase().includes(query))
      .sort((left, right) => {
        let comparison = 0;
        if (sort === 'name') comparison = left.displayName.localeCompare(right.displayName);
        if (sort === 'size') comparison = compareOptionalBigInt(left.sizeBytes, right.sizeBytes);
        if (sort === 'modified') comparison = (left.modifiedAt ?? '').localeCompare(right.modifiedAt ?? '');
        return descending ? -comparison : comparison;
      });
  }, [listing, filter, showHidden, sort, descending]);
  const selectedEntries = (listing?.entries ?? []).filter((entry) => selected.has(entry.path));

  function owner(session = sftp) {
    if (!session) throw new Error('SFTP unavailable');
    return session;
  }
  function toggle(path: string, checked: boolean) {
    setSelected((current) => { const next = new Set(current); if (checked) next.add(path); else next.delete(path); return next; });
  }
  async function upload() {
    const session = owner(); const target = listing?.path; if (!target) return;
    setError(null);
    try { const grant = await filesApi.chooseUploadFiles(); if (!grant) return; setPlans([await filesApi.planUpload(session, grant.id, target, conflictPolicy)]); }
    catch (reason) { setError(applicationError(reason).message); }
  }
  async function download() {
    const session = owner(); const paths = selectedEntries.filter((entry) => entry.kind === 'file').map((entry) => entry.path); if (!paths.length) return;
    setError(null);
    try { const grant = await filesApi.chooseDownloadDirectory(); if (!grant) return; setPlans([await filesApi.planDownload(session, grant.id, paths, conflictPolicy)]); }
    catch (reason) { setError(applicationError(reason).message); }
  }
  async function prepareDelete() {
    const session = owner(); setError(null);
    try { const next: FileOperationPlan[] = []; for (const entry of selectedEntries) next.push(await filesApi.planDelete(session, entry.path)); setPlans(next); }
    catch (reason) { setError(applicationError(reason).message); }
  }
  async function prepareEntryDialog() {
    const session = owner(); if (!dialog || !listing) return;
    setError(null);
    try {
      const plan = dialog.type === 'mkdir'
        ? await filesApi.planCreateDirectory(session, listing.path, dialog.value)
        : await filesApi.planRename(session, dialog.entry.path, dialog.value);
      setDialog(null); setPlans([plan]);
    } catch (reason) { setError(applicationError(reason).message); }
  }
  async function executePlans() {
    const session = owner(); const accepted = plans; setPlans([]); setError(null);
    try { for (const plan of accepted) await filesApi.execute(session, plan.id); if (listing) await loadDirectory(session, listing.path, 'history'); }
    catch (reason) { setError(applicationError(reason).message); }
  }
  async function showProperties(entry: RemoteEntry) {
    try { setProperties(await filesApi.properties(owner(), entry.path)); } catch (reason) { setError(applicationError(reason).message); }
  }
  async function retry(job: TransferJob) {
    try { setPlans([await filesApi.planRetry(owner(), job.id)]); } catch (reason) { setError(applicationError(reason).message); }
  }
  function visitHistory(index: number) { const target = history[index]; if (!sftp || !target) return; setHistoryIndex(index); void loadDirectory(sftp, target, 'history'); }
  function up() { if (!listing || !sftp || listing.path === '/') return; const target = listing.path.replace(/\/+$/, '').replace(/\/[^/]*$/, '') || '/'; void loadDirectory(sftp, target); }

  if (!visible) return null;
  return (
    <section className="files-workspace" aria-label={`Files on ${host.displayName}`}>
      <header className="files-heading">
        <div><div className="eyebrow">SFTP WORKSPACE</div><h1>{host.displayName} files</h1></div>
        <div className="files-session">{sftp ? `SFTP v${sftp.protocolVersion}` : connected ? 'Opening SFTP…' : 'Disconnected'}</div>
      </header>
      {!connected && <Notice>Connect this host to browse files. {listing && 'The visible listing is stale and mutations are disabled.'}<div className="notice-action"><Button onClick={onShowOverview}>Open overview</Button></div></Notice>}
      {stale && connected && <Notice>This listing belongs to an old connection and is read-only until refresh completes.</Notice>}
      {error && <Notice>{error}</Notice>}
      <div className="files-toolbar" aria-label="File navigation">
        <Button variant="ghost" disabled={historyIndex <= 0 || loading} aria-label="Back" onClick={() => visitHistory(historyIndex - 1)}>←</Button>
        <Button variant="ghost" disabled={historyIndex < 0 || historyIndex >= history.length - 1 || loading} aria-label="Forward" onClick={() => visitHistory(historyIndex + 1)}>→</Button>
        <Button variant="ghost" disabled={!listing || listing.path === '/' || loading} aria-label="Up" onClick={up}>↑</Button>
        <Button variant="ghost" disabled={!sftp || !listing || loading} aria-label="Refresh" onClick={() => { if (sftp && listing) void loadDirectory(sftp, listing.path, 'history'); }}>↻</Button>
        <form className="files-address" onSubmit={(event) => { event.preventDefault(); if (sftp) void loadDirectory(sftp, address); }}>
          <label className="sr-only" htmlFor={`files-address-${host.id}`}>Remote path</label>
          <input id={`files-address-${host.id}`} value={address} onChange={(event) => setAddress(event.target.value)} spellCheck={false} />
          <Button type="submit" disabled={!sftp || loading}>Go</Button>
        </form>
      </div>
      <div className="files-actions">
        <Button disabled={!canMutate} onClick={() => void upload()}>Upload files…</Button>
        <Button disabled={!canMutate || !selectedEntries.some((entry) => entry.kind === 'file')} onClick={() => void download()}>Download selected…</Button>
        <Button disabled={!canMutate} onClick={() => setDialog({ type: 'mkdir', value: '' })}>New directory</Button>
        <Button disabled={!canMutate || selectedEntries.length !== 1 || selectedEntries[0]?.kind !== 'file'} onClick={() => { const entry = selectedEntries[0]; if (entry) setDialog({ type: 'rename', entry, value: entry.name }); }}>Rename</Button>
        <Button variant="danger" disabled={!canMutate || selectedEntries.length === 0} onClick={() => void prepareDelete()}>Delete</Button>
        <label>Conflict <select value={conflictPolicy} onChange={(event) => setConflictPolicy(event.target.value as ConflictPolicy)}><option value="skip">Skip</option><option value="keepBoth">Keep both</option><option value="replace">Replace</option></select></label>
      </div>
      <div className="files-view-options">
        <label><input type="checkbox" checked={showHidden} onChange={(event) => setShowHidden(event.target.checked)} /> Show hidden</label>
        <label>Filter loaded entries <input value={filter} onChange={(event) => setFilter(event.target.value)} /></label>
        <span>{listing ? `${listing.entries.length}${listing.partial ? '+' : ''} loaded` : 'No listing'}</span>
      </div>
      <div className="files-table-wrap">
        {loading && !listing ? <Spinner label="Loading remote directory…" /> : (
          <table className="files-table">
            <thead><tr><th><span className="sr-only">Select</span></th><th><button onClick={() => { if (sort === 'name') setDescending(!descending); else { setSort('name'); setDescending(false); } }}>Name</button></th><th>Type</th><th><button onClick={() => { if (sort === 'size') setDescending(!descending); else { setSort('size'); setDescending(false); } }}>Size</button></th><th><button onClick={() => { if (sort === 'modified') setDescending(!descending); else { setSort('modified'); setDescending(false); } }}>Modified</button></th><th>Mode</th><th><span className="sr-only">Actions</span></th></tr></thead>
            <tbody>{entries.map((entry) => <tr key={entry.path} className={selected.has(entry.path) ? 'is-selected' : ''} onDoubleClick={() => { if (entry.kind === 'directory' && sftp) void loadDirectory(sftp, entry.path); }}>
              <td><input aria-label={`Select ${entry.displayName}`} type="checkbox" checked={selected.has(entry.path)} onChange={(event) => toggle(entry.path, event.target.checked)} /></td>
              <td><button className="file-name" onClick={() => toggle(entry.path, !selected.has(entry.path))} onKeyDown={(event) => { if (event.key === 'Enter' && entry.kind === 'directory' && sftp) void loadDirectory(sftp, entry.path); }}>{entry.kind === 'directory' ? '📁' : entry.kind === 'symlink' ? '↗' : '▧'} {entry.displayName}</button></td>
              <td>{entry.kind}</td><td>{entry.sizeBytes ?? 'Unknown'}</td><td>{entry.modifiedAt ? new Date(entry.modifiedAt).toLocaleString() : 'Unknown'}</td><td>{entry.permissions ?? 'Unknown'}</td>
              <td><button onClick={() => void showProperties(entry)}>Properties</button></td>
            </tr>)}</tbody>
          </table>
        )}
        {listing?.partial && <Notice>Partial listing: the safety cap of {listing.entryCap} entries was reached. Sorting and filtering apply only to loaded entries.</Notice>}
      </div>
      <section className="transfer-queue" aria-label="Transfer queue"><h2>Transfers</h2>{transfers.length === 0 ? <p>No transfers in this session.</p> : transfers.map((job) => <div className="transfer-row" key={job.id}><div><strong>{job.direction === 'upload' ? '↑' : '↓'} {job.sourceDisplay}</strong><span> → {job.destinationDisplay}</span></div><div><span className={`transfer-state transfer-state--${job.state}`}>{job.state}</span> <span>{formatProgress(job)}</span>{['queued','preparing','transferring'].includes(job.state) && <button onClick={() => { if (sftp) void filesApi.cancel(sftp, job.id); }}>Cancel</button>}{job.retryable && <button onClick={() => void retry(job)}>Prepare retry</button>}</div>{job.error && <span className="transfer-error">{job.error.message}</span>}</div>)}</section>
      {dialog && <Modal title={dialog.type === 'mkdir' ? 'Create remote directory' : 'Rename remote entry'} onClose={() => setDialog(null)}><label className="field"><span>Name</span><input autoFocus value={dialog.value} onChange={(event) => setDialog({ ...dialog, value: event.target.value })} /></label><p className="dialog-description">The operation will be prepared as an immutable plan. Rename never overwrites an existing destination.</p><div className="modal-actions"><Button onClick={() => setDialog(null)}>Cancel</Button><Button disabled={!dialog.value} onClick={() => void prepareEntryDialog()}>Review plan</Button></div></Modal>}
      {plans.length > 0 && <Modal title="Approve file operation" onClose={() => setPlans([])}><p className="dialog-description">Host: <strong>{host.displayName}</strong>. This one-time approval expires at {new Date(plans[0]!.expiresAt).toLocaleTimeString()}.</p>{plans.map((plan) => <div className="file-plan" key={plan.id}><strong>{plan.kind} · {plan.risk}{plan.conflictPolicy ? ` · Conflict: ${plan.conflictPolicy}` : ''}</strong>{plan.items.map((item, index) => <div key={index}><code>{item.sourceDisplay}</code> → <code>{item.destinationDisplay}</code>{item.sizeBytes && ` · ${item.sizeBytes} bytes`}</div>)}</div>)}{plans.some((plan) => plan.kind === 'delete') && <Notice>Delete is permanent. Directories must be empty; symbolic links are removed without following their target.</Notice>}<div className="modal-actions"><Button onClick={() => setPlans([])}>Cancel</Button><Button variant={plans.some((plan) => plan.kind === 'delete') ? 'danger' : 'primary'} onClick={() => void executePlans()}>Approve and execute</Button></div></Modal>}
      {properties && <Modal title="Remote properties" onClose={() => setProperties(null)}><dl className="properties"><dt>Name</dt><dd>{properties.displayName}</dd><dt>Type</dt><dd>{properties.kind}</dd><dt>Size</dt><dd>{properties.sizeBytes ?? 'Unknown'}</dd><dt>Modified</dt><dd>{properties.modifiedAt ?? 'Unknown'}</dd><dt>Permissions</dt><dd>{properties.permissions ?? 'Unknown'}</dd><dt>UID / GID</dt><dd>{properties.uid ?? 'Unknown'} / {properties.gid ?? 'Unknown'}</dd></dl><div className="modal-actions"><Button onClick={() => setProperties(null)}>Close</Button></div></Modal>}
    </section>
  );
}

function compareOptionalBigInt(left: string | null, right: string | null) { if (left === right) return 0; if (left === null) return 1; if (right === null) return -1; const a = BigInt(left); const b = BigInt(right); return a < b ? -1 : a > b ? 1 : 0; }
function formatProgress(job: TransferJob) { if (!job.totalBytes) return `${job.confirmedBytes} bytes`; const confirmed = BigInt(job.confirmedBytes); const total = BigInt(job.totalBytes); const percent = total === 0n ? 100n : (confirmed * 100n) / total; return `${confirmed} / ${total} bytes (${percent}%)`; }
