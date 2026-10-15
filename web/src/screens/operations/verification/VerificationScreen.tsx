import { createSignal, For, onCleanup, onMount, Show, type JSX } from 'solid-js';
import { ApiClient, ApiError } from '../../../shared/api/client';
import type { MeView } from '../../../shared/api/generated/me';
import type {
  VerificationCaseAuditRow,
  VerificationCaseDetail,
  VerificationCaseDocumentRow,
  VerificationCaseFilter,
  VerificationCaseViewAnswer,
} from '../../../shared/api/generated/verification_case';
import type {
  VerificationCaseApproveInput,
  VerificationCaseApproveOutput,
} from '../../../shared/api/generated/verification_case_approve';
import type {
  VerificationCaseRejectInput,
  VerificationCaseRejectOutput,
} from '../../../shared/api/generated/verification_case_reject';
import type {
  VerificationCaseRequestUpdateInput,
  VerificationCaseRequestUpdateOutput,
} from '../../../shared/api/generated/verification_case_request_update';
import type {
  VerificationCaseRevokeInput,
  VerificationCaseRevokeOutput,
} from '../../../shared/api/generated/verification_case_revoke';
import type {
  VerificationCaseStartReviewInput,
  VerificationCaseStartReviewOutput,
} from '../../../shared/api/generated/verification_case_start_review';
import type {
  VerificationDocumentLinkInput,
  VerificationDocumentLinkOutput,
} from '../../../shared/api/generated/verification_document_link';
import type { VerificationPolicyData } from '../../../shared/api/generated/verification_policy';
import type {
  VerificationQueueFilter,
  VerificationQueueItem,
} from '../../../shared/api/generated/verification_queue';
import { screenT } from '../../../shared/i18n';
import { wordedError } from '../../../shared/i18n/worded-error';
import { can } from '../../staff/parts/can';
import { en, th } from './i18n';
import './verification.css';

const REVIEW_PERMISSION = 'VerificationReview';
const QUEUE_POLL_MS = 30_000;
const ROUTE_BRANCH_CASES = '/api/operations/verification/branch-cases';

export const S5_REASON_CODES = [
  'DOCUMENT_UNREADABLE',
  'DOCUMENT_EXPIRED',
  'DETAILS_MISMATCH',
  'DOCUMENT_NOT_ACCEPTED',
  'SUSPECTED_FRAUD',
  'OTHER',
] as const;

export type S5ReasonCode = (typeof S5_REASON_CODES)[number];

type PendingReasonAction = 'reject' | 'request-update' | 'revoke';

function makeIdempotencyKey(): string {
  if (typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function') {
    return crypto.randomUUID();
  }
  return 'xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx'.replace(/[xy]/g, (c) => {
    const r = (Math.random() * 16) | 0;
    const v = c === 'x' ? r : (r & 0x3) | 0x8;
    return v.toString(16);
  });
}

function todayBangkokIso(): string {
  try {
    return new Intl.DateTimeFormat('en-CA', {
      timeZone: 'Asia/Bangkok',
      year: 'numeric',
      month: '2-digit',
      day: '2-digit',
    }).format(new Date());
  } catch {
    return new Date().toISOString().slice(0, 10);
  }
}

async function parseRawResponseError(res: Response): Promise<never> {
  const text = await res.text();
  try {
    const json = JSON.parse(text);
    if (json && typeof json === 'object' && json.error) {
      if (typeof json.error === 'string') {
        throw new ApiError({
          code: 'refused',
          message: json.error,
          status: res.status,
        });
      }
      throw new ApiError({
        code: json.error.code ?? 'unknown',
        field: json.error.field,
        reason: typeof json.error.reason === 'string' ? json.error.reason : undefined,
        message: json.error.message ?? text,
        status: res.status,
      });
    }
  } catch (e) {
    if (e instanceof ApiError) throw e;
  }
  throw new ApiError({
    code: 'httpError',
    message: text || res.statusText,
    status: res.status,
  });
}

export function VerificationScreen(): JSX.Element {
  const st = screenT('operations.verification', { th, en });
  const api = new ApiClient();

  const [authorized, setAuthorized] = createSignal<boolean | null>(null);
  const [refusalMessage, setRefusalMessage] = createSignal<string | null>(null);
  const [queueItems, setQueueItems] = createSignal<VerificationQueueItem[]>([]);
  const [selectedCaseId, setSelectedCaseId] = createSignal<string | null>(null);
  const [selectedCase, setSelectedCase] = createSignal<VerificationCaseDetail | null>(null);
  const [caseDocuments, setCaseDocuments] = createSignal<VerificationCaseDocumentRow[]>([]);
  const [caseAudit, setCaseAudit] = createSignal<VerificationCaseAuditRow[]>([]);

  const [previewDocId, setPreviewDocId] = createSignal<string | null>(null);
  const [previewUrl, setPreviewUrl] = createSignal<string | null>(null);
  const [linkExpiredDocId, setLinkExpiredDocId] = createSignal<string | null>(null);
  const [docError, setDocError] = createSignal<string | null>(null);

  const [expiresAt, setExpiresAt] = createSignal<string>('');
  const [reasonCode, setReasonCode] = createSignal<S5ReasonCode>('DOCUMENT_UNREADABLE');
  const [pendingReasonAction, setPendingReasonAction] = createSignal<PendingReasonAction | null>(null);

  const [errorMessage, setErrorMessage] = createSignal<string | null>(null);
  const [retryFn, setRetryFn] = createSignal<(() => Promise<void>) | null>(null);

  const [branchReviewAllowed, setBranchReviewAllowed] = createSignal<boolean>(false);
  const [branchCustomerId, setBranchCustomerId] = createSignal<string>('');
  const [branchDocType, setBranchDocType] = createSignal<'PASSPORT' | 'NATIONAL_ID'>('PASSPORT');
  const [branchFile, setBranchFile] = createSignal<File | null>(null);
  const [branchSuccessMsg, setBranchSuccessMsg] = createSignal<string | null>(null);

  let lastQueueVersion: number | undefined;
  let expiresInputRef: HTMLInputElement | undefined;
  let reasonSelectRef: HTMLSelectElement | undefined;
  let branchCustInputRef: HTMLInputElement | undefined;
  let branchDocTypeSelectRef: HTMLSelectElement | undefined;
  let branchFileInputRef: HTMLInputElement | undefined;
  const idemKeyMap = new Map<string, string>();

  function getOrCreateIdemKey(signature: string): string {
    const existing = idemKeyMap.get(signature);
    if (existing) return existing;
    const created = makeIdempotencyKey();
    idemKeyMap.set(signature, created);
    return created;
  }

  function clearIdemKey(signature: string): void {
    idemKeyMap.delete(signature);
  }

  function describeFailure(err: unknown): string {
    if (err instanceof ApiError && err.status < 500) {
      if (err.code === 'staleVersion') return st('errStaleVersion');
      if (err.code === 'idempotencyConflict') return st('errIdempotencyConflict');
      if (err.code === 'outOfScope') return st('errOutOfScope');
      if (err.code === 'validationFailed') return st('errValidationFailed');
      if (err.code === 'linkExpired') return st('errLinkExpired');
      if (err.code === 'linkRefused') return st('errLinkRefused');
      if (err.code === 'branchReviewNotAllowed') return st('errBranchReviewNotAllowed');
    }
    return wordedError(err);
  }

  function reasonLabel(code: S5ReasonCode): string {
    switch (code) {
      case 'DOCUMENT_UNREADABLE':
        return st('reasonDocumentUnreadable');
      case 'DOCUMENT_EXPIRED':
        return st('reasonDocumentExpired');
      case 'DETAILS_MISMATCH':
        return st('reasonDetailsMismatch');
      case 'DOCUMENT_NOT_ACCEPTED':
        return st('reasonDocumentNotAccepted');
      case 'SUSPECTED_FRAUD':
        return st('reasonSuspectedFraud');
      case 'OTHER':
        return st('reasonOther');
    }
  }

  async function fetchQueue(isPoll = false): Promise<void> {
    try {
      const filter: VerificationQueueFilter = {
        since: isPoll && lastQueueVersion !== undefined ? lastQueueVersion : null,
      };
      const answer = await api.view<VerificationQueueItem[], VerificationQueueFilter>(
        'verification_queue',
        filter.since !== null ? { since: filter.since } : {},
      );
      lastQueueVersion = answer.version;
      if (answer.changed && Array.isArray(answer.data)) {
        setQueueItems(answer.data);
      }
      setAuthorized(true);
      setRefusalMessage(null);
    } catch (err: unknown) {
      if (
        err instanceof ApiError &&
        (err.code === 'roleNotAllowed' ||
          err.code === 'roleNotAllowedHere' ||
          err.code === 'outOfScope' ||
          err.status === 403)
      ) {
        setQueueItems([]);
        setAuthorized(false);
        setRefusalMessage(describeFailure(err));
        return;
      }
      setErrorMessage(describeFailure(err));
    }
  }

  async function loadPolicy(): Promise<void> {
    try {
      const policyAns = await api.view<VerificationPolicyData>('verification_policy');
      setBranchReviewAllowed(policyAns.data?.branchReviewAllowed === true);
    } catch {
      setBranchReviewAllowed(false);
    }
  }

  async function initScreen(): Promise<void> {
    void loadPolicy();
    try {
      const meAns = await api.view<MeView>('me');
      const meData = meAns.data;
      if (meData) {
        if (!can(meData, REVIEW_PERMISSION)) {
          setQueueItems([]);
          setAuthorized(false);
          setRefusalMessage(st('refused'));
          return;
        }
      }
    } catch (err: unknown) {
      if (
        err instanceof ApiError &&
        (err.code === 'roleNotAllowed' ||
          err.code === 'roleNotAllowedHere' ||
          err.code === 'outOfScope')
      ) {
        setQueueItems([]);
        setAuthorized(false);
        setRefusalMessage(describeFailure(err));
        return;
      }
    }
    await fetchQueue(false);
  }

  onMount(() => {
    const timer = setInterval(() => {
      if (authorized() === true) {
        void fetchQueue(true);
      }
    }, QUEUE_POLL_MS);
    onCleanup(() => clearInterval(timer));
    void initScreen();
  });

  async function loadCase(caseId: string, keepError = false): Promise<void> {
    setSelectedCaseId(caseId);
    if (!keepError) {
      setErrorMessage(null);
      setRetryFn(null);
      setDocError(null);
      setLinkExpiredDocId(null);
      setPreviewDocId(null);
      setPreviewUrl(null);
      setPendingReasonAction(null);
    }
    try {
      const answer = await api.view<VerificationCaseViewAnswer, VerificationCaseFilter>(
        'verification_case',
        { filter: { id: caseId } },
      );
      if (answer.data) {
        setSelectedCase(answer.data.case);
        setCaseDocuments(answer.data.documents ?? []);
        setCaseAudit(answer.data.audit ?? []);
      }
    } catch (err: unknown) {
      setErrorMessage(describeFailure(err));
    }
  }

  async function openDocument(docId: string): Promise<void> {
    const currentCase = selectedCase();
    if (!currentCase) return;

    setDocError(null);
    setLinkExpiredDocId(null);
    const key = makeIdempotencyKey();
    const input: VerificationDocumentLinkInput = {
      id: docId,
      expectedVersion: currentCase.version,
    };

    try {
      const linkOut = await api.command<VerificationDocumentLinkInput, VerificationDocumentLinkOutput>(
        'POST',
        `/api/operations/verification/documents/${encodeURIComponent(docId)}/link`,
        input,
        key,
      );
      const contentRes = await fetch(linkOut.url, {
        method: 'GET',
        credentials: 'same-origin',
      });
      if (!contentRes.ok) {
        await parseRawResponseError(contentRes);
      }
      setPreviewDocId(docId);
      setPreviewUrl(linkOut.url);
    } catch (err: unknown) {
      const msg = describeFailure(err);
      if (
        err instanceof ApiError &&
        (err.status === 410 || err.code === 'linkExpired' || err.code === 'linkRefused')
      ) {
        setPreviewUrl(null);
        setLinkExpiredDocId(docId);
        setDocError(msg);
        return;
      }
      setDocError(msg);
    }
  }

  async function handleCommandFailure(
    err: unknown,
    caseId: string,
    retryCallback: () => Promise<void>,
  ): Promise<void> {
    const msg = describeFailure(err);
    setErrorMessage(msg);
    if (err instanceof ApiError && err.code === 'staleVersion') {
      setRetryFn(null);
      await loadCase(caseId, true);
      return;
    }
    setRetryFn(() => retryCallback);
  }

  async function handleStartReview(): Promise<void> {
    const currentCase = selectedCase();
    if (!currentCase) return;

    const caseId = currentCase.id;
    const expectedVersion = currentCase.version;
    const sig = `${caseId}:start-review:${expectedVersion}`;
    const key = getOrCreateIdemKey(sig);
    const body: VerificationCaseStartReviewInput = {
      id: caseId,
      expectedVersion,
    };

    const execute = async (): Promise<void> => {
      try {
        setErrorMessage(null);
        const updated = await api.command<
          VerificationCaseStartReviewInput,
          VerificationCaseStartReviewOutput
        >(
          'POST',
          `/api/operations/verification/cases/${encodeURIComponent(caseId)}/start-review`,
          body,
          key,
        );
        clearIdemKey(sig);
        setRetryFn(null);
        setSelectedCase(updated);
      } catch (err: unknown) {
        await handleCommandFailure(err, caseId, execute);
      }
    };

    await execute();
  }

  async function handleApprove(): Promise<void> {
    const currentCase = selectedCase();
    if (!currentCase) return;

    const rawDate = (expiresInputRef ? expiresInputRef.value : expiresAt()).trim();
    const today = todayBangkokIso();
    if (!rawDate || !/^\d{4}-\d{2}-\d{2}$/.test(rawDate) || rawDate <= today) {
      setErrorMessage(st('errExpiresAtRequired'));
      return;
    }

    const caseId = currentCase.id;
    const expectedVersion = currentCase.version;
    const sig = `${caseId}:approve:${expectedVersion}:${rawDate}`;
    const key = getOrCreateIdemKey(sig);
    const body: VerificationCaseApproveInput = {
      id: caseId,
      expectedVersion,
      expiresAt: rawDate,
    };

    const execute = async (): Promise<void> => {
      try {
        setErrorMessage(null);
        const updated = await api.command<
          VerificationCaseApproveInput,
          VerificationCaseApproveOutput
        >(
          'POST',
          `/api/operations/verification/cases/${encodeURIComponent(caseId)}/approve`,
          body,
          key,
        );
        clearIdemKey(sig);
        setRetryFn(null);
        setSelectedCase(updated);
      } catch (err: unknown) {
        await handleCommandFailure(err, caseId, execute);
      }
    };

    await execute();
  }

  function currentReasonCode(): S5ReasonCode {
    const domVal = reasonSelectRef?.value as S5ReasonCode | undefined;
    if (domVal && (S5_REASON_CODES as readonly string[]).includes(domVal)) {
      return domVal;
    }
    return reasonCode();
  }

  async function submitReasonDecision(action: PendingReasonAction): Promise<void> {
    const currentCase = selectedCase();
    if (!currentCase) return;

    const selectedReason = currentReasonCode();
    if (!selectedReason || !(S5_REASON_CODES as readonly string[]).includes(selectedReason)) {
      setErrorMessage(st('errReasonRequired'));
      return;
    }

    const caseId = currentCase.id;
    const expectedVersion = currentCase.version;
    const sig = `${caseId}:${action}:${expectedVersion}:${selectedReason}`;
    const key = getOrCreateIdemKey(sig);

    const execute = async (): Promise<void> => {
      try {
        setErrorMessage(null);
        if (action === 'reject') {
          const body: VerificationCaseRejectInput = {
            id: caseId,
            expectedVersion,
            reasonCode: selectedReason,
          };
          const updated = await api.command<
            VerificationCaseRejectInput,
            VerificationCaseRejectOutput
          >(
            'POST',
            `/api/operations/verification/cases/${encodeURIComponent(caseId)}/reject`,
            body,
            key,
          );
          setSelectedCase(updated);
        } else if (action === 'request-update') {
          const body: VerificationCaseRequestUpdateInput = {
            id: caseId,
            expectedVersion,
            reasonCode: selectedReason,
          };
          const updated = await api.command<
            VerificationCaseRequestUpdateInput,
            VerificationCaseRequestUpdateOutput
          >(
            'POST',
            `/api/operations/verification/cases/${encodeURIComponent(caseId)}/request-update`,
            body,
            key,
          );
          setSelectedCase(updated);
        } else {
          const body: VerificationCaseRevokeInput = {
            id: caseId,
            expectedVersion,
            reasonCode: selectedReason,
          };
          const updated = await api.command<
            VerificationCaseRevokeInput,
            VerificationCaseRevokeOutput
          >(
            'POST',
            `/api/operations/verification/cases/${encodeURIComponent(caseId)}/revoke`,
            body,
            key,
          );
          setSelectedCase(updated);
        }
        clearIdemKey(sig);
        setRetryFn(null);
        setPendingReasonAction(null);
      } catch (err: unknown) {
        await handleCommandFailure(err, caseId, execute);
      }
    };

    await execute();
  }

  async function handleBranchSubmit(e?: Event): Promise<void> {
    e?.preventDefault();
    setBranchSuccessMsg(null);
    setErrorMessage(null);

    const custId = (branchCustInputRef ? branchCustInputRef.value : branchCustomerId()).trim();
    const docType = (
      branchDocTypeSelectRef ? branchDocTypeSelectRef.value : branchDocType()
    ) as 'PASSPORT' | 'NATIONAL_ID';
    const file = branchFileInputRef?.files?.[0] ?? branchFile();
    if (!custId || !file) {
      setErrorMessage(st('errBranchFormIncomplete'));
      return;
    }

    const sig = `branch-intake:${custId}:${docType}:${file.name}:${file.size}`;
    const key = getOrCreateIdemKey(sig);

    const formData = new FormData();
    formData.append('customerId', custId);
    formData.append('documentType', docType);
    formData.append('file', file);

    try {
      await api.postForm(ROUTE_BRANCH_CASES, formData, key);
      clearIdemKey(sig);
      setBranchSuccessMsg(st('branchSubmittedNotice'));
      setBranchCustomerId('');
      setBranchFile(null);
      await fetchQueue(false);
    } catch (err: unknown) {
      if (err instanceof ApiError && err.code === 'branchReviewNotAllowed') {
        setBranchReviewAllowed(false);
      }
      setErrorMessage(describeFailure(err));
    }
  }

  function renderAuditSummary(ev: VerificationCaseAuditRow): string {
    const raw = ev as unknown as {
      beforeState?: Record<string, unknown> | null;
      afterState?: Record<string, unknown> | null;
      role?: string;
      createdAt?: string;
    };
    const beforeObj = (ev.before ?? raw.beforeState) as Record<string, unknown> | null | undefined;
    const afterObj = (ev.after ?? raw.afterState) as Record<string, unknown> | null | undefined;
    const beforeStatus = beforeObj && typeof beforeObj.status === 'string' ? beforeObj.status : '';
    const afterStatus = afterObj && typeof afterObj.status === 'string' ? afterObj.status : '';
    const transition =
      beforeStatus || afterStatus ? ` (${beforeStatus || '-'} -> ${afterStatus || '-'})` : '';
    const actorRole = ev.actorRole ?? raw.role ?? '';
    const timestamp = ev.at ?? raw.createdAt ?? '';
    return `${ev.action}${transition} · ${actorRole} · ${timestamp}`;
  }

  return (
    <div class="xc-verification-screen" data-testid="verification-screen">
      <Show when={authorized() === false || refusalMessage() !== null}>
        <div data-refused class="xc-verification-error" role="alert">
          {refusalMessage() ?? st('refused')}
        </div>
      </Show>

      <Show when={errorMessage()}>
        {(msg) => (
          <div data-error class="xc-verification-error" role="alert">
            <span>{msg()}</span>
            <Show when={retryFn()}>
              {(fn) => (
                <button
                  type="button"
                  data-action="retry"
                  onClick={() => {
                    const retry = fn();
                    void retry();
                  }}
                >
                  {st('actionRetry')}
                </button>
              )}
            </Show>
          </div>
        )}
      </Show>

      <Show when={authorized() === true}>
        <div class="xc-verification-grid">
          <section class="xc-verification-panel" aria-label={st('queueTitle')}>
            <div class="xc-verification-header">
              <h2>{st('queueTitle')}</h2>
              <button
                type="button"
                data-action="refresh"
                onClick={() => void fetchQueue(true)}
              >
                {st('refresh')}
              </button>
            </div>

            <Show
              when={queueItems().length > 0}
              fallback={<p data-testid="queue-empty">{st('queueEmpty')}</p>}
            >
              <table class="xc-verification-table">
                <thead>
                  <tr>
                    <th>{st('colCustomer')}</th>
                    <th>{st('colIdentityLast4')}</th>
                    <th>{st('colStatus')}</th>
                    <th>{st('colBranch')}</th>
                    <th>{st('colSubmittedAt')}</th>
                  </tr>
                </thead>
                <tbody>
                  <For each={queueItems()}>
                    {(item) => (
                      <tr
                        data-case-row
                        data-case-id={item.id}
                        class={selectedCaseId() === item.id ? 'is-selected' : ''}
                        onClick={() => void loadCase(item.id)}
                      >
                        <td>{item.customerDisplayName}</td>
                        <td>{item.identityLast4 ?? '-'}</td>
                        <td>{item.status}</td>
                        <td>{item.branchId ?? st('branchOrgWide')}</td>
                        <td>{item.submittedAt ?? '-'}</td>
                      </tr>
                    )}
                  </For>
                </tbody>
              </table>
            </Show>
          </section>

          <section class="xc-verification-panel" aria-label={st('caseTitle')}>
            <h2>{st('caseTitle')}</h2>
            <Show
              when={selectedCase()}
              fallback={<p data-testid="case-select-prompt">{st('caseSelectPrompt')}</p>}
            >
              {(detail) => {
                const extra = () =>
                  detail() as unknown as {
                    customerDisplayName?: string;
                    identityLast4?: string;
                  };
                return (
                  <div data-testid="case-detail-view">
                    <dl>
                      <dt>{st('caseIdLabel')}</dt>
                      <dd>{detail().id}</dd>
                      <Show when={extra().customerDisplayName}>
                        <dt>{st('colCustomer')}</dt>
                        <dd>{extra().customerDisplayName}</dd>
                      </Show>
                      <Show when={extra().identityLast4}>
                        <dt>{st('colIdentityLast4')}</dt>
                        <dd>{extra().identityLast4}</dd>
                      </Show>
                      <dt>{st('caseStatusLabel')}</dt>
                      <dd data-testid="case-status">{detail().status}</dd>
                      <dt>{st('caseVersionLabel')}</dt>
                      <dd>{detail().version}</dd>
                      <Show when={detail().expiresAt}>
                        <dt>{st('caseExpiresAtLabel')}</dt>
                        <dd>{detail().expiresAt}</dd>
                      </Show>
                      <Show when={detail().decisionReasonCode}>
                        <dt>{st('caseReasonLabel')}</dt>
                        <dd>{detail().decisionReasonCode}</dd>
                      </Show>
                    </dl>

                    <h3>{st('documentsTitle')}</h3>
                    <Show
                      when={caseDocuments().length > 0}
                      fallback={<p>{st('documentsEmpty')}</p>}
                    >
                      <ul>
                        <For each={caseDocuments()}>
                          {(doc) => (
                            <li>
                              <span>
                                {doc.documentType} ({doc.mime}, {doc.sizeBytes} B)
                              </span>{' '}
                              <button
                                type="button"
                                data-doc-id={doc.id}
                                data-action="view-document"
                                onClick={() => void openDocument(doc.id)}
                              >
                                {st('docView')}
                              </button>
                            </li>
                          )}
                        </For>
                      </ul>
                    </Show>

                    <Show when={docError()}>
                      {(dErr) => (
                        <div data-error class="xc-verification-error" role="alert">
                          <span>{dErr()}</span>
                          <Show when={linkExpiredDocId()}>
                            {(expiredId) => (
                              <button
                                type="button"
                                data-action="refresh-link"
                                onClick={() => void openDocument(expiredId())}
                              >
                                {st('docRefreshLink')}
                              </button>
                            )}
                          </Show>
                        </div>
                      )}
                    </Show>

                    <Show when={previewUrl()}>
                      {(url) => (
                        <div class="xc-verification-doc-preview">
                          <img src={url()} alt={previewDocId() ?? st('documentsTitle')} />
                        </div>
                      )}
                    </Show>

                    <h3>{st('auditTitle')}</h3>
                    <Show
                      when={caseAudit().length > 0}
                      fallback={<p>{st('auditEmpty')}</p>}
                    >
                      <ul data-testid="case-audit-list">
                        <For each={caseAudit()}>
                          {(ev) => <li>{renderAuditSummary(ev)}</li>}
                        </For>
                      </ul>
                    </Show>

                    <div class="xc-verification-decisions">
                      <h3>{st('decisionsTitle')}</h3>

                      <div class="xc-verification-decision-buttons">
                        <Show when={detail().status === 'SUBMITTED'}>
                          <button
                            type="button"
                            data-action="start-review"
                            onClick={() => void handleStartReview()}
                          >
                            {st('actionStartReview')}
                          </button>
                        </Show>

                        <Show
                          when={
                            detail().status === 'UNDER_REVIEW' ||
                            detail().status === 'SUBMITTED'
                          }
                        >
                          <label>
                            <span>{st('expiresAtInputLabel')}</span>
                            <input
                              ref={(el) => {
                                expiresInputRef = el;
                              }}
                              type="date"
                              name="expiresAt"
                              value={expiresAt()}
                              onInput={(e) => setExpiresAt(e.currentTarget.value)}
                            />
                          </label>
                          <button
                            type="button"
                            data-action="approve"
                            onClick={() => void handleApprove()}
                          >
                            {st('actionApprove')}
                          </button>
                          <button
                            type="button"
                            data-action="reject"
                            onClick={() => setPendingReasonAction('reject')}
                          >
                            {st('actionReject')}
                          </button>
                          <button
                            type="button"
                            data-action="request-update"
                            onClick={() => setPendingReasonAction('request-update')}
                          >
                            {st('actionRequestUpdate')}
                          </button>
                        </Show>

                        <Show when={detail().status === 'VERIFIED'}>
                          <button
                            type="button"
                            data-action="revoke"
                            onClick={() => setPendingReasonAction('revoke')}
                          >
                            {st('actionRevoke')}
                          </button>
                        </Show>
                      </div>

                      <Show when={pendingReasonAction()}>
                        {(action) => (
                          <div data-testid="reason-decision-panel">
                            <label>
                              <span>{st('reasonLabel')}</span>
                              <select
                                ref={(el) => {
                                  reasonSelectRef = el;
                                }}
                                name="reasonCode"
                                value={reasonCode()}
                                onChange={(e) =>
                                  setReasonCode(e.currentTarget.value as S5ReasonCode)
                                }
                              >
                                <For each={S5_REASON_CODES}>
                                  {(code) => (
                                    <option value={code}>{reasonLabel(code)}</option>
                                  )}
                                </For>
                              </select>
                            </label>

                            <Show when={action() === 'reject'}>
                              <button
                                type="button"
                                data-action="confirm-reject"
                                onClick={() => void submitReasonDecision('reject')}
                              >
                                {st('actionConfirmReject')}
                              </button>
                            </Show>
                            <Show when={action() === 'request-update'}>
                              <button
                                type="button"
                                data-action="confirm-request-update"
                                onClick={() => void submitReasonDecision('request-update')}
                              >
                                {st('actionConfirmRequestUpdate')}
                              </button>
                            </Show>
                            <Show when={action() === 'revoke'}>
                              <button
                                type="button"
                                data-action="confirm-revoke"
                                onClick={() => void submitReasonDecision('revoke')}
                              >
                                {st('actionConfirmRevoke')}
                              </button>
                            </Show>
                            <button
                              type="button"
                              data-action="cancel-reason"
                              onClick={() => setPendingReasonAction(null)}
                            >
                              {st('actionCancel')}
                            </button>
                          </div>
                        )}
                      </Show>
                    </div>
                  </div>
                );
              }}
            </Show>
          </section>
        </div>

        <Show
          when={branchReviewAllowed()}
          fallback={
            <p class="xc-verification-panel" data-testid="branch-verification-disabled">
              {st('branchReviewDisabled')}
            </p>
          }
        >
          <section
            class="xc-verification-panel"
            data-testid="branch-verification-form"
            aria-label={st('branchFormTitle')}
          >
            <h2>{st('branchFormTitle')}</h2>
            <form onSubmit={(e) => void handleBranchSubmit(e)}>
              <div>
                <label>
                  <span>{st('branchCustomerIdLabel')}</span>
                  <input
                    ref={(el) => {
                      branchCustInputRef = el;
                    }}
                    type="text"
                    name="customerId"
                    value={branchCustomerId()}
                    onInput={(e) => setBranchCustomerId(e.currentTarget.value)}
                  />
                </label>
              </div>
              <div>
                <label>
                  <span>{st('branchDocTypeLabel')}</span>
                  <select
                    ref={(el) => {
                      branchDocTypeSelectRef = el;
                    }}
                    name="documentType"
                    value={branchDocType()}
                    onChange={(e) =>
                      setBranchDocType(e.currentTarget.value as 'PASSPORT' | 'NATIONAL_ID')
                    }
                  >
                    <option value="PASSPORT">PASSPORT</option>
                    <option value="NATIONAL_ID">NATIONAL_ID</option>
                  </select>
                </label>
              </div>
              <div>
                <label>
                  <span>{st('branchFileLabel')}</span>
                  <input
                    ref={(el) => {
                      branchFileInputRef = el;
                    }}
                    type="file"
                    name="file"
                    accept="image/jpeg,image/png,application/pdf"
                    onChange={(e) => setBranchFile(e.currentTarget.files?.[0] ?? null)}
                  />
                </label>
              </div>
              <button
                type="submit"
                data-action="submit-branch-case"
                onClick={(e) => void handleBranchSubmit(e)}
              >
                {st('branchSubmitBtn')}
              </button>
            </form>
            <Show when={branchSuccessMsg()}>
              {(msg) => <p data-testid="branch-case-success">{msg()}</p>}
            </Show>
          </section>
        </Show>
      </Show>
    </div>
  );
}
