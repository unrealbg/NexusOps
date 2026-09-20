import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import type { FileOperationPlan, Host, RemoteTextDocument, SftpSessionInfo, TransferJob } from '@nexusops/protocol';
import { Button, Modal, Notice } from '@nexusops/ui';
import { applicationError, filesApi } from '../../api/client';

function owns(session: SftpSessionInfo | null, document: RemoteTextDocument) {
  return !!session && session.hostId === document.hostId &&
    session.hostSessionId === document.hostSessionId && session.id === document.sftpSessionId;
}
function displayPath(path: string) {
  return [...path].map((character) => {
    const code = character.codePointAt(0)!;
    return code < 32 || code === 127 || code === 0x061c || code === 0x200e || code === 0x200f ||
      (code >= 0x202a && code <= 0x202e) || (code >= 0x2066 && code <= 0x2069)
      ? `\\u{${code.toString(16).toUpperCase().padStart(4, '0')}}` : character;
  }).join('');
}

export function RemoteTextEditor({ initial, host, session, connected, visible, transfers, onClose }:
  { initial: RemoteTextDocument; host: Host; session: SftpSessionInfo | null; connected: boolean; visible: boolean; transfers: TransferJob[]; onClose: () => void }) {
  const [document, setDocument] = useState(initial);
  const [text, setText] = useState(initial.text);
  const [baseline, setBaseline] = useState(initial.text);
  const [revisionStale, setRevisionStale] = useState(false);
  const [approval, setApproval] = useState<{ session: SftpSessionInfo; plan: FileOperationPlan; text: string } | null>(null);
  const [jobId, setJobId] = useState<string | null>(null);
  const [executing, setExecuting] = useState(false);
  const [planning, setPlanning] = useState(false);
  const [reloading, setReloading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [confirm, setConfirm] = useState<'close' | 'reload' | null>(null);
  const planningGuard = useRef(false);
  const submittedText = useRef<string | null>(null);
  const approvalRef = useRef<typeof approval>(null);
  const generation = useRef(0);
  const scopeRef = useRef({ session, connected, visible });
  useLayoutEffect(() => { scopeRef.current = { session, connected, visible }; }, [session, connected, visible]);
  const dirty = text !== baseline;
  const authorityLive = connected && owns(session, document);
  const stale = !authorityLive || revisionStale;
  const canPlan = visible && authorityLive && !revisionStale && dirty && !planning && !approval && !jobId && !executing && !reloading;

  function forgetApproval() {
    const pending = approvalRef.current;
    approvalRef.current = null;
    setApproval(null);
    if (pending) {
      setRevisionStale(true); // The document token was consumed when its plan was created.
      void filesApi.discardPlan(pending.session, pending.plan.id).catch(() => undefined);
    }
  }

  useEffect(() => {
    if (authorityLive && visible) return;
    generation.current += 1;
    planningGuard.current = false;
    let active = true;
    void Promise.resolve().then(() => {
      if (!active) return;
      setPlanning(false);
      setReloading(false);
      forgetApproval();
    });
    return () => { active = false; };
  }, [authorityLive, document.id, visible]);

  useEffect(() => {
    if (!jobId) return;
    const job = transfers.find((value) => value.id === jobId);
    if (!job) return;
    if (job.state === 'completed') {
      if (submittedText.current !== null) setBaseline(submittedText.current);
      submittedText.current = null;
      setRevisionStale(true); // A new open is required before another save.
      setJobId(null);
    } else if (['failed', 'cancelled', 'outcomeUnknown'].includes(job.state)) {
      setRevisionStale(true);
      setError(job.error?.message ?? `Editor save ended as ${job.state}. Reload before another save.`);
      setJobId(null);
    }
  }, [transfers, jobId]);

  async function reviewSave() {
    if (planningGuard.current || !canPlan || !session) return;
    planningGuard.current = true;
    setPlanning(true);
    const requestGeneration = ++generation.current;
    const requestDocument = document;
    const requestText = text;
    const owner = session;
    setError(null);
    try {
      const plan = await filesApi.planTextSave(owner, requestDocument.id, requestText);
      const current = scopeRef.current;
      if (generation.current !== requestGeneration || !current.connected || !current.visible ||
          !owns(current.session, requestDocument) || requestDocument.id !== document.id) {
        void filesApi.discardPlan(owner, plan.id).catch(() => undefined);
        return;
      }
      const next = { session: owner, plan, text: requestText };
      approvalRef.current = next;
      setApproval(next);
    } catch (reason) {
      if (generation.current === requestGeneration) {
        const failure = applicationError(reason);
        setError(failure.message);
        setRevisionStale(true); // Planning consumes the document token even when it fails.
      }
    } finally {
      if (generation.current === requestGeneration) {
        planningGuard.current = false;
        setPlanning(false);
      }
    }
  }

  async function approve() {
    const pending = approvalRef.current;
    if (!pending) return;
    approvalRef.current = null;
    setApproval(null);
    const current = scopeRef.current;
    if (!current.connected || !owns(current.session, document) ||
        current.session?.id !== pending.session.id) {
      void filesApi.discardPlan(pending.session, pending.plan.id).catch(() => undefined);
      return;
    }
    try {
      submittedText.current = pending.text;
      setExecuting(true);
      const jobs = await filesApi.execute(pending.session, pending.plan.id);
      if (jobs.length !== 1) throw new Error('The editor save did not return one transfer job.');
      setJobId(jobs[0]!.id);
    } catch (reason) {
      setRevisionStale(true);
      setError(applicationError(reason).message);
    } finally { setExecuting(false); }
  }

  async function reload() {
    const current = scopeRef.current;
    if (!current.connected || !current.session || !visible) return;
    const requestGeneration = ++generation.current;
    forgetApproval();
    setError(null);
    setReloading(true);
    try {
      const next = await filesApi.openText(current.session, document.path);
      const latest = scopeRef.current;
      if (generation.current !== requestGeneration || !latest.connected || !latest.visible || !owns(latest.session, next)) return;
      setDocument(next);
      setText(next.text);
      setBaseline(next.text);
      setRevisionStale(false);
      setJobId(null);
    } catch (reason) { if (generation.current === requestGeneration) setError(applicationError(reason).message); }
    finally { if (generation.current === requestGeneration) setReloading(false); }
  }

  function close() {
    if (dirty) { setConfirm('close'); return; }
    forgetApproval();
    onClose();
  }

  if (!visible) return null;
  return <section className="remote-text-editor" aria-label="Remote text editor">
    <header><h2>{displayPath(document.path.split('/').at(-1) ?? document.path)}</h2><span>{dirty ? 'Unsaved changes' : 'No unsaved changes'}</span></header>
    <p>Host: {host.displayName} · Path: <code>{displayPath(document.path)}</code></p>
    <p>UTF-8 · {document.newline === 'crLf' ? 'CRLF' : 'LF'} · {document.bom ? 'BOM' : 'No BOM'} · 1 MiB maximum</p>
    {stale && <Notice>Remote authority is stale or disconnected. Local text is retained; reload the remote file before saving.</Notice>}
    {jobId && <Notice>Saving staged text; wait for the transfer result.</Notice>}
    {error && <Notice>{error}</Notice>}
    <label className="field"><span>Remote text</span><textarea aria-label="Remote text" spellCheck={false} value={text} disabled={!!jobId || !!approval || executing || reloading}
      onChange={(event) => setText(event.target.value)}
      onKeyDown={(event) => {
        if (event.code === 'KeyS' && event.ctrlKey && !event.altKey && !event.metaKey && !event.repeat && canPlan) {
          event.preventDefault(); void reviewSave();
        }
      }} /></label>
    <div className="modal-actions">
      <Button disabled={!canPlan} onClick={() => void reviewSave()}>Save / review</Button>
      <Button disabled={!connected || !session || !!jobId || executing || reloading} onClick={() => { if (dirty) setConfirm('reload'); else void reload(); }}>Reload remote</Button>
      <Button disabled={!!jobId || executing || reloading} onClick={close}>Close</Button>
    </div>
    {approval && authorityLive && <Modal title="Approve remote text save" onClose={forgetApproval}>
      <p>Host: {host.displayName}. Edit / replace content at <code>{displayPath(document.path)}</code>.</p>
      <p>Risk: {approval.plan.risk}. Original: {document.originalBytes} bytes. New: {approval.plan.items[0]?.sizeBytes} bytes.</p>
      <p>The one-time plan expires at {new Date(approval.plan.expiresAt).toLocaleTimeString()}.</p>
      <div className="modal-actions"><Button onClick={forgetApproval}>Cancel</Button><Button onClick={() => void approve()}>Approve and save</Button></div>
    </Modal>}
    {confirm && <Modal title={confirm === 'close' ? 'Discard unsaved changes?' : 'Reload remote text?'} onClose={() => setConfirm(null)}>
      <p>Unsaved local text will be lost.</p>
      <div className="modal-actions"><Button onClick={() => setConfirm(null)}>Keep editing</Button>
        <Button variant="danger" onClick={() => { const action = confirm; setConfirm(null); if (action === 'close') { forgetApproval(); onClose(); } else void reload(); }}>Discard changes</Button></div>
    </Modal>}
  </section>;
}
