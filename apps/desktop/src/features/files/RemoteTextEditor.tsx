import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import type { FileOperationPlan, Host, RemoteTextDocument, SftpSessionInfo, TransferJob } from '@nexusops/protocol';
import { Button, Modal, Notice } from '@nexusops/ui';
import { applicationError, filesApi } from '../../api/client';

function owns(session: SftpSessionInfo | null, document: RemoteTextDocument) {
  return !!session && session.hostId === document.hostId &&
    session.hostSessionId === document.hostSessionId && session.id === document.sftpSessionId;
}
function documentOwner(document: RemoteTextDocument) {
  return { hostId: document.hostId, hostSessionId: document.hostSessionId, id: document.sftpSessionId };
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
  const [approval, setApproval] = useState<{ session: SftpSessionInfo; plan: FileOperationPlan; text: string; bufferGeneration: number } | null>(null);
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
  const bufferGeneration = useRef(0);
  const textRef = useRef(initial.text);
  const documentRef = useRef(initial);
  const retired = useRef(false);
  const mounted = useRef(true);
  const discardedPlans = useRef(new Set<string>());
  const discardedDocuments = useRef(new Set<string>());
  const confirmationRef = useRef<'close' | 'reload' | null>(null);
  const textAreaRef = useRef<HTMLTextAreaElement>(null);
  const scopeRef = useRef({ session, connected, visible });
  useLayoutEffect(() => { scopeRef.current = { session, connected, visible }; }, [session, connected, visible]);
  useEffect(() => {
    if (visible && !confirmationRef.current && !approvalRef.current) textAreaRef.current?.focus();
  }, [visible]);
  const dirty = text !== baseline;
  const authorityLive = connected && owns(session, document);
  const stale = !authorityLive || revisionStale;
  const canPlan = visible && authorityLive && !revisionStale && dirty && !planning && !approval && !jobId && !executing && !reloading && !confirm;

  function discardPlanOnce(owner: SftpSessionInfo, planId: string) {
    if (discardedPlans.current.has(planId)) return;
    discardedPlans.current.add(planId);
    void filesApi.discardPlan(owner, planId).catch(() => undefined);
  }
  function discardDocumentOnce(value: RemoteTextDocument) {
    if (discardedDocuments.current.has(value.id)) return;
    discardedDocuments.current.add(value.id);
    void filesApi.discardTextDocument(documentOwner(value), value.id).catch(() => undefined);
  }

  function forgetApproval(updateState = true) {
    const pending = approvalRef.current;
    approvalRef.current = null;
    if (updateState && mounted.current) setApproval(null);
    if (pending) {
      if (updateState && mounted.current) setRevisionStale(true); // Planning consumed the document token.
      discardPlanOnce(pending.session, pending.plan.id);
    }
  }

  function retire() {
    if (retired.current) return;
    retired.current = true;
    generation.current += 1;
    planningGuard.current = false;
    forgetApproval(false);
    discardDocumentOnce(documentRef.current);
  }

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      generation.current += 1;
      // React development StrictMode may replay effects without unmounting.
      void Promise.resolve().then(() => { if (!mounted.current) retire(); });
    };
    // Retirement uses the latest document/approval refs; the lifetime effect must not restart.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

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
    // The approval ref is intentionally read at the session transition, not captured on render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
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
    if (planningGuard.current || !canPlan || !session || !mounted.current || retired.current) return;
    planningGuard.current = true;
    setPlanning(true);
    const requestGeneration = ++generation.current;
    const requestDocument = document;
    const requestText = textRef.current;
    const requestBufferGeneration = bufferGeneration.current;
    const owner = session;
    setError(null);
    try {
      const plan = await filesApi.planTextSave(owner, requestDocument.id, requestText);
      const current = scopeRef.current;
      if (!mounted.current || retired.current || generation.current !== requestGeneration ||
          !current.connected || !current.visible || !owns(current.session, requestDocument) ||
          requestDocument.id !== documentRef.current.id || confirmationRef.current ||
          textRef.current !== requestText || bufferGeneration.current !== requestBufferGeneration) {
        discardPlanOnce(owner, plan.id);
        if (mounted.current && !retired.current && generation.current === requestGeneration) {
          setRevisionStale(true);
          setError('The text changed while save review was prepared. Reload the remote file before saving.');
        }
        return;
      }
      const next = { session: owner, plan, text: requestText, bufferGeneration: requestBufferGeneration };
      approvalRef.current = next;
      setApproval(next);
    } catch (reason) {
      if (mounted.current && !retired.current && generation.current === requestGeneration) {
        const failure = applicationError(reason);
        setError(failure.message);
        setRevisionStale(true); // Planning consumes the document token even when it fails.
      }
    } finally {
      if (mounted.current && !retired.current && generation.current === requestGeneration) {
        planningGuard.current = false;
        setPlanning(false);
      }
    }
  }

  async function approve() {
    const pending = approvalRef.current;
    if (!pending) return;
    approvalRef.current = null;
    if (mounted.current) setApproval(null);
    const current = scopeRef.current;
    if (!mounted.current || retired.current || !current.connected || !current.visible ||
        !owns(current.session, documentRef.current) || current.session?.id !== pending.session.id ||
        pending.text !== textRef.current || pending.bufferGeneration !== bufferGeneration.current) {
      discardPlanOnce(pending.session, pending.plan.id);
      if (mounted.current) setRevisionStale(true);
      return;
    }
    try {
      submittedText.current = pending.text;
      setExecuting(true);
      const jobs = await filesApi.execute(pending.session, pending.plan.id);
      if (!mounted.current || retired.current) return;
      if (jobs.length !== 1) throw new Error('The editor save did not return one transfer job.');
      setJobId(jobs[0]!.id);
    } catch (reason) {
      if (mounted.current && !retired.current) {
        setRevisionStale(true);
        setError(applicationError(reason).message);
      }
    } finally { if (mounted.current && !retired.current) setExecuting(false); }
  }

  async function reload() {
    const current = scopeRef.current;
    if (!current.connected || !current.session || !visible || retired.current || !mounted.current) return;
    const requestGeneration = ++generation.current;
    forgetApproval();
    setError(null);
    setReloading(true);
    try {
      const oldDocument = documentRef.current;
      const next = await filesApi.openText(current.session, oldDocument.path);
      const latest = scopeRef.current;
      if (!mounted.current || retired.current || generation.current !== requestGeneration ||
          !latest.connected || !latest.visible || !owns(latest.session, next)) {
        discardDocumentOnce(next);
        return;
      }
      discardDocumentOnce(oldDocument);
      documentRef.current = next;
      textRef.current = next.text;
      bufferGeneration.current += 1;
      setDocument(next);
      setText(next.text);
      setBaseline(next.text);
      setRevisionStale(false);
      setJobId(null);
    } catch (reason) { if (mounted.current && !retired.current && generation.current === requestGeneration) setError(applicationError(reason).message); }
    finally { if (mounted.current && !retired.current && generation.current === requestGeneration) setReloading(false); }
  }

  function close() {
    if (dirty) { confirmationRef.current = 'close'; setConfirm('close'); return; }
    retire();
    onClose();
  }

  if (!visible) return null;
  return <section className="remote-text-editor" aria-label="Remote text editor"
    onKeyDown={(event) => {
      if (event.code === 'KeyS' && event.ctrlKey && !event.altKey && !event.metaKey &&
          !event.getModifierState('AltGraph') && !event.repeat && canPlan &&
          !planningGuard.current && !confirmationRef.current) {
        event.preventDefault(); void reviewSave();
      }
    }}>
    <header><h2>{displayPath(document.path.split('/').at(-1) ?? document.path)}</h2><span>{dirty ? 'Unsaved changes' : 'No unsaved changes'}</span></header>
    <p>Host: {host.displayName} · Path: <code>{displayPath(document.path)}</code></p>
    <p>UTF-8 · {document.newline === 'crLf' ? 'CRLF' : 'LF'} · {document.bom ? 'BOM' : 'No BOM'} · 1 MiB maximum</p>
    {stale && <Notice>Remote authority is stale or disconnected. Local text is retained; reload the remote file before saving.</Notice>}
    {jobId && <Notice>Saving staged text; wait for the transfer result.</Notice>}
    {error && <Notice>{error}</Notice>}
    <label className="field"><span>Remote text</span><textarea ref={textAreaRef} aria-label="Remote text" spellCheck={false} value={text} disabled={!!jobId || !!approval || executing || reloading}
      onChange={(event) => { textRef.current = event.target.value; bufferGeneration.current += 1; setText(event.target.value); }} /></label>
    <div className="modal-actions">
      <Button disabled={!canPlan} onClick={() => void reviewSave()}>Save / review</Button>
      <Button disabled={!connected || !session || !!jobId || executing || reloading} onClick={() => { if (dirty) { confirmationRef.current = 'reload'; setConfirm('reload'); } else void reload(); }}>Reload remote</Button>
      <Button disabled={!!jobId || executing || reloading} onClick={close}>Close</Button>
    </div>
    {approval && authorityLive && <Modal title="Approve remote text save" onClose={() => forgetApproval()}>
      <p>Host: {host.displayName}. Edit / replace content at <code>{displayPath(document.path)}</code>.</p>
      <p>Risk: {approval.plan.risk}. Original: {document.originalBytes} bytes. New: {approval.plan.items[0]?.sizeBytes} bytes.</p>
      <p>The one-time plan expires at {new Date(approval.plan.expiresAt).toLocaleTimeString()}.</p>
      <div className="modal-actions"><Button onClick={() => forgetApproval()}>Cancel</Button><Button onClick={() => void approve()}>Approve and save</Button></div>
    </Modal>}
    {confirm && <Modal title={confirm === 'close' ? 'Discard unsaved changes?' : 'Reload remote text?'} onClose={() => { confirmationRef.current = null; setConfirm(null); }}>
      <p>Unsaved local text will be lost.</p>
      <div className="modal-actions"><Button onClick={() => { confirmationRef.current = null; setConfirm(null); }}>Keep editing</Button>
        <Button variant="danger" onClick={() => { const action = confirm; confirmationRef.current = null; setConfirm(null); if (action === 'close') { retire(); onClose(); } else void reload(); }}>Discard changes</Button></div>
    </Modal>}
  </section>;
}
