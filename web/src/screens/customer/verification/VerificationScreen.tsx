import { For, Show, createSignal, onMount, type JSX } from 'solid-js';
import { ApiError } from '../../../shared/api/client';
import { screenT } from '../../../shared/i18n';
import { isServerFailure, wordedError } from '../../../shared/i18n/worded-error';
import * as table from './i18n';
import {
  fetchMyVerification,
  newIdempotencyKey,
  openVerificationCase,
  removeVerificationDocument,
  requestMyDocumentLink,
  submitVerificationCase,
  uploadVerificationDocument,
  type MyVerificationCaseDetail,
  type VerificationDocumentType,
  type VerificationMyDocumentLinkOutput,
} from './verificationApi';
import './verification.css';

const t = screenT('customer.verification', table);

const bangkokDateFmt = new Intl.DateTimeFormat('en-CA', {
  timeZone: 'Asia/Bangkok',
  year: 'numeric',
  month: '2-digit',
  day: '2-digit',
});

function formatExpiresAt(raw: string): string {
  if (!raw || /^\d{4}-\d{2}-\d{2}$/.test(raw)) return raw;
  const d = new Date(raw);
  if (Number.isNaN(d.getTime())) return raw;
  return bangkokDateFmt.format(d);
}

function formatRefusalError(err: unknown): string {
  if (err instanceof ApiError) {
    if (err.code === 'outOfScope') {
      return t('outOfScope');
    }
    if (err.code === 'roleNotAllowed' || err.code === 'roleNotAllowedHere') {
      return t('roleNotAllowed');
    }
    if (err.code === 'documentTypeNotAccepted') {
      return t('documentTypeNotAccepted');
    }
    if (err.code === 'documentsMissing') {
      return t('documentsMissing');
    }
    if (err.code === 'caseLocked') {
      return t('caseLocked');
    }
    const msg = wordedError(err);
    if (msg.toLowerCase().includes(err.code.toLowerCase())) {
      return t('roleNotAllowed');
    }
    return msg;
  }
  return wordedError(err);
}

function formatUploadError(err: unknown): string {
  if (err instanceof ApiError) {
    if (err.code === 'outOfScope') {
      return t('outOfScope');
    }
    if (err.code === 'roleNotAllowed' || err.code === 'roleNotAllowedHere') {
      return t('roleNotAllowed');
    }
    if (err.code === 'documentTypeNotAccepted') {
      const rawMsg = err.message?.trim() ?? '';
      if (rawMsg && !rawMsg.toLowerCase().includes(err.code.toLowerCase())) {
        return `${t('documentTypeNotAccepted')} (${rawMsg})`;
      }
      return t('documentTypeNotAccepted');
    }
    if (isServerFailure(err) || err.status === 401) {
      return wordedError(err);
    }
    const rawMsg = err.message?.trim() ?? '';
    if (rawMsg && !rawMsg.toLowerCase().includes(err.code.toLowerCase())) {
      return rawMsg;
    }
    return t('uploadFailed');
  }
  return t('uploadFailed');
}

export function VerificationScreen(): JSX.Element {
  const [checkingAuth, setCheckingAuth] = createSignal(true);
  const [permitted, setPermitted] = createSignal(false);
  const [loading, setLoading] = createSignal(false);
  const [errorMsg, setErrorMsg] = createSignal<string | null>(null);
  const [actionError, setActionError] = createSignal<string | null>(null);
  const [readiness, setReadiness] = createSignal<string>('NONE');
  const [currentCase, setCurrentCase] = createSignal<MyVerificationCaseDetail | null>(null);

  const [docType, setDocType] = createSignal<VerificationDocumentType>('PASSPORT');
  const [selectedFile, setSelectedFile] = createSignal<File | null>(null);
  const [pendingUploadKey, setPendingUploadKey] = createSignal<string | null>(null);
  const [uploading, setUploading] = createSignal(false);
  const [uploadError, setUploadError] = createSignal<string | null>(null);
  const [busyAction, setBusyAction] = createSignal(false);
  const [docLinks, setDocLinks] = createSignal<Record<string, VerificationMyDocumentLinkOutput>>({});

  let fileInputEl: HTMLInputElement | undefined;

  const refreshVerification = async () => {
    const ans = await fetchMyVerification();
    setReadiness(ans.readiness ?? 'NONE');
    setCurrentCase(ans.case ?? null);
  };

  onMount(() => {
    void (async () => {
      setCheckingAuth(true);
      setLoading(true);
      setErrorMsg(null);
      try {
        await refreshVerification();
        setPermitted(true);
      } catch (err) {
        if (err instanceof ApiError && err.code === 'outOfScope') {
          setPermitted(false);
        } else {
          setPermitted(true);
          setErrorMsg(formatRefusalError(err));
        }
      } finally {
        setLoading(false);
        setCheckingAuth(false);
      }
    })();
  });

  const activeStep = (): number => {
    const c = currentCase();
    if (!c) return 1;
    switch (c.status) {
      case 'DRAFT':
        return 2;
      case 'SUBMITTED':
      case 'UNDER_REVIEW':
        return 3;
      case 'VERIFIED':
      case 'NEEDS_UPDATE':
      case 'REJECTED':
      case 'EXPIRED':
      case 'REVOKED':
        return 4;
      default:
        return 1;
    }
  };

  const handleOpenCase = async () => {
    if (busyAction()) return;
    setBusyAction(true);
    setActionError(null);
    try {
      await openVerificationCase({ reason: null });
      await refreshVerification();
      setUploadError(null);
      setPendingUploadKey(null);
      setSelectedFile(null);
    } catch (err) {
      setActionError(formatRefusalError(err));
    } finally {
      setBusyAction(false);
    }
  };

  let succeededKey: string | null = null;

  const handleUpload = async (isRetry: boolean) => {
    const c = currentCase();
    if (!c) return;
    if (!isRetry && uploading()) return;
    const file = selectedFile() ?? fileInputEl?.files?.[0] ?? null;
    if (!file) return;

    let key = pendingUploadKey();
    if (!key) {
      key = newIdempotencyKey();
      setPendingUploadKey(key);
    }

    setUploading(true);
    if (!isRetry) {
      setUploadError(null);
    }
    try {
      await uploadVerificationDocument(c.id, docType(), file, key);
      succeededKey = key;
      setPendingUploadKey(null);
      setUploadError(null);
      await refreshVerification();
    } catch (err) {
      if (succeededKey !== key) {
        setUploadError(formatUploadError(err));
      }
    } finally {
      setUploading(false);
    }
  };

  const handleRemoveDocument = async (docId: string) => {
    if (busyAction()) return;
    setBusyAction(true);
    setActionError(null);
    try {
      await removeVerificationDocument({ id: docId });
      await refreshVerification();
    } catch (err) {
      setActionError(formatRefusalError(err));
    } finally {
      setBusyAction(false);
    }
  };

  const handleSubmitCase = async (caseId: string) => {
    if (busyAction()) return;
    setBusyAction(true);
    setActionError(null);
    try {
      await submitVerificationCase({ id: caseId });
      await refreshVerification();
    } catch (err) {
      setActionError(formatRefusalError(err));
    } finally {
      setBusyAction(false);
    }
  };

  const handleViewDocument = async (docId: string) => {
    setActionError(null);
    try {
      const link = await requestMyDocumentLink({ id: docId });
      setDocLinks((prev) => ({ ...prev, [docId]: link }));
    } catch (err) {
      setActionError(formatRefusalError(err));
    }
  };

  const isEditableCase = (status: string): boolean =>
    status === 'DRAFT' || status === 'NEEDS_UPDATE';

  const isWaitingCase = (status: string): boolean =>
    status === 'SUBMITTED' || status === 'UNDER_REVIEW';

  return (
    <>
      <Show when={!checkingAuth() && !permitted()}>
        <div class="xc-verification-blocked" role="alert">
          <p>{errorMsg() ?? t('accessDenied')}</p>
        </div>
      </Show>

      <Show when={permitted()}>
        <div
          class="xc-verification-screen"
          data-screen="verification"
          data-verification-screen="true"
          data-readiness={readiness()}
        >
          <header class="xc-verification-header">
            <h1 class="xc-verification-title">{t('title')}</h1>
            <p class="xc-verification-subtitle">{t('subtitle')}</p>
          </header>

          <ol class="xc-verification-steps" data-verification-steps="true">
            <li
              class={`xc-verification-step ${activeStep() === 1 ? 'xc-verification-step--active' : ''}`}
              data-step="1"
            >
              <span>1. {t('stepSelect')}</span>
            </li>
            <li
              class={`xc-verification-step ${activeStep() === 2 ? 'xc-verification-step--active' : ''}`}
              data-step="2"
            >
              <span>2. {t('stepSubmit')}</span>
            </li>
            <li
              class={`xc-verification-step ${activeStep() === 3 ? 'xc-verification-step--active' : ''}`}
              data-step="3"
            >
              <span>3. {t('stepReview')}</span>
            </li>
            <li
              class={`xc-verification-step ${activeStep() === 4 ? 'xc-verification-step--active' : ''}`}
              data-step="4"
            >
              <span>4. {t('stepResult')}</span>
            </li>
          </ol>

          <Show when={loading()}>
            <div class="xc-verification-loading" role="status">
              {t('loading')}
            </div>
          </Show>

          <Show when={!loading() && errorMsg()}>
            {(msg) => (
              <div class="xc-verification-error" role="alert">
                <p class="xc-verification-error-message">{msg()}</p>
              </div>
            )}
          </Show>

          <Show when={!loading() && !errorMsg()}>
            <Show when={actionError()}>
              {(msg) => (
                <div
                  class="xc-verification-error"
                  data-action-error="true"
                  role="alert"
                >
                  <p class="xc-verification-error-message">{msg()}</p>
                </div>
              )}
            </Show>

            {/* Step 1: No case yet (readiness: NONE, case: null) */}
            <Show when={currentCase() === null}>
              <section class="xc-verification-card" data-case-state="NONE">
                <h2 class="xc-verification-card-title">{t('startTitle')}</h2>
                <p class="xc-verification-card-desc">{t('startDesc')}</p>
                <div class="xc-verification-field">
                  <label class="xc-verification-label" for="xc-verification-doctype-init">
                    {t('docTypeLabel')}
                  </label>
                  <select
                    id="xc-verification-doctype-init"
                    class="xc-verification-select"
                    data-field="documentType"
                    value={docType()}
                    onChange={(e) =>
                      setDocType(e.currentTarget.value as VerificationDocumentType)
                    }
                  >
                    <option value="PASSPORT">{t('docTypePassport')}</option>
                    <option value="NATIONAL_ID">{t('docTypeNationalId')}</option>
                  </select>
                </div>
                <div class="xc-verification-actions">
                  <button
                    type="button"
                    class="xc-verification-btn"
                    data-action="start"
                    disabled={busyAction()}
                    onClick={() => void handleOpenCase()}
                  >
                    {t('startButton')}
                  </button>
                </div>
              </section>
            </Show>

            <Show when={currentCase()}>
              {(c) => (
                <>
                  {/* DRAFT or NEEDS_UPDATE: same-case upload, remove, submit/resubmit */}
                  <Show when={isEditableCase(c().status)}>
                    <section class="xc-verification-card" data-case-state={c().status}>
                      <h2 class="xc-verification-card-title">
                        {c().status === 'NEEDS_UPDATE' ? t('needsUpdateTitle') : t('draftTitle')}
                      </h2>
                      <p class="xc-verification-card-desc">
                        {c().status === 'NEEDS_UPDATE' ? t('needsUpdateDesc') : t('draftDesc')}
                      </p>

                      <Show when={c().decisionReasonCode}>
                        {(reason) => (
                          <div class="xc-verification-reason" role="note">
                            <span class="xc-verification-reason-label">{t('reasonLabel')}</span>
                            <p class="xc-verification-reason-text" data-decision-reason="true">
                              {reason()}
                            </p>
                          </div>
                        )}
                      </Show>

                      <div class="xc-verification-field">
                        <label class="xc-verification-label" for="xc-verification-doctype">
                          {t('docTypeLabel')}
                        </label>
                        <select
                          id="xc-verification-doctype"
                          class="xc-verification-select"
                          data-field="documentType"
                          value={docType()}
                          onChange={(e) => {
                            setDocType(e.currentTarget.value as VerificationDocumentType);
                            setPendingUploadKey(null);
                            setUploadError(null);
                          }}
                        >
                          <option value="PASSPORT">{t('docTypePassport')}</option>
                          <option value="NATIONAL_ID">{t('docTypeNationalId')}</option>
                        </select>
                      </div>

                      <div class="xc-verification-field">
                        <label class="xc-verification-label" for="xc-verification-file">
                          {t('fileLabel')}
                        </label>
                        <input
                          ref={(el) => {
                            fileInputEl = el;
                          }}
                          id="xc-verification-file"
                          type="file"
                          class="xc-verification-file-input"
                          accept="image/jpeg,image/png,application/pdf"
                          onChange={(e) => {
                            const f = e.currentTarget.files?.[0] ?? null;
                            setSelectedFile(f);
                            setPendingUploadKey(null);
                            setUploadError(null);
                          }}
                        />
                        <div class="xc-verification-actions">
                          <button
                            type="button"
                            class="xc-verification-btn"
                            data-action="upload"
                            disabled={uploading()}
                            onClick={() => void handleUpload(false)}
                          >
                            {uploading() ? t('uploading') : t('uploadButton')}
                          </button>
                          <Show when={Boolean(uploadError() || pendingUploadKey())}>
                            <button
                              type="button"
                              class="xc-verification-btn"
                              data-action="retry"
                              onClick={() => void handleUpload(true)}
                            >
                              {t('retryUploadButton')}
                            </button>
                          </Show>
                        </div>

                        <Show when={uploadError()}>
                          {(errText) => (
                            <div
                              class="xc-verification-upload-error"
                              data-upload-error="true"
                              role="alert"
                            >
                              <p class="xc-verification-error-message">{errText()}</p>
                            </div>
                          )}
                        </Show>
                      </div>

                      <div class="xc-verification-docs">
                        <h3 class="xc-verification-label">{t('uploadedDocsTitle')}</h3>
                        <Show
                          when={c().documents.length > 0}
                          fallback={<p class="xc-verification-card-desc">{t('noDocsYet')}</p>}
                        >
                          <ul class="xc-verification-doc-list">
                            <For each={c().documents}>
                              {(doc) => (
                                <li class="xc-verification-doc-item" data-doc-id={doc.id}>
                                  <div class="xc-verification-doc-meta">
                                    <span>{doc.documentType}</span>
                                    <span>{doc.mime}</span>
                                  </div>
                                  <div class="xc-verification-actions">
                                    <button
                                      type="button"
                                      class="xc-verification-btn"
                                      data-action="view-document"
                                      data-view-doc={doc.id}
                                      onClick={() => void handleViewDocument(doc.id)}
                                    >
                                      {t('viewDocButton')}
                                    </button>
                                    <Show when={docLinks()[doc.id]}>
                                      {(link) => (
                                        <a
                                          href={link().url}
                                          target="_blank"
                                          rel="noopener noreferrer"
                                        >
                                          {t('openDocLink')}
                                        </a>
                                      )}
                                    </Show>
                                    <button
                                      type="button"
                                      class="xc-verification-btn"
                                      data-action="remove"
                                      data-remove-doc={doc.id}
                                      disabled={busyAction()}
                                      onClick={() => void handleRemoveDocument(doc.id)}
                                    >
                                      {t('removeDocButton')}
                                    </button>
                                  </div>
                                </li>
                              )}
                            </For>
                          </ul>
                        </Show>
                      </div>

                      <div class="xc-verification-actions">
                        <Show
                          when={c().status === 'NEEDS_UPDATE'}
                          fallback={
                            <button
                              type="button"
                              class="xc-verification-btn"
                              data-action="submit"
                              disabled={busyAction()}
                              onClick={() => void handleSubmitCase(c().id)}
                            >
                              {t('submitButton')}
                            </button>
                          }
                        >
                          <button
                            type="button"
                            class="xc-verification-btn"
                            data-action="resubmit"
                            disabled={busyAction()}
                            onClick={() => void handleSubmitCase(c().id)}
                          >
                            {t('resubmitButton')}
                          </button>
                        </Show>
                      </div>
                    </section>
                  </Show>

                  {/* SUBMITTED or UNDER_REVIEW: waiting state, edits frozen */}
                  <Show when={isWaitingCase(c().status)}>
                    <section class="xc-verification-card" data-case-state={c().status}>
                      <h2 class="xc-verification-card-title">{t('waitingTitle')}</h2>
                      <p class="xc-verification-card-desc">{t('waitingDesc')}</p>
                      <Show when={c().documents.length > 0}>
                        <ul class="xc-verification-doc-list">
                          <For each={c().documents}>
                            {(doc) => (
                              <li class="xc-verification-doc-item" data-doc-id={doc.id}>
                                <div class="xc-verification-doc-meta">
                                  <span>{doc.documentType}</span>
                                  <span>{doc.mime}</span>
                                </div>
                                <div class="xc-verification-actions">
                                  <button
                                    type="button"
                                    class="xc-verification-btn"
                                    data-action="view-document"
                                    data-view-doc={doc.id}
                                    onClick={() => void handleViewDocument(doc.id)}
                                  >
                                    {t('viewDocButton')}
                                  </button>
                                  <Show when={docLinks()[doc.id]}>
                                    {(link) => (
                                      <a
                                        href={link().url}
                                        target="_blank"
                                        rel="noopener noreferrer"
                                      >
                                        {t('openDocLink')}
                                      </a>
                                    )}
                                  </Show>
                                </div>
                              </li>
                            )}
                          </For>
                        </ul>
                      </Show>
                    </section>
                  </Show>

                  {/* VERIFIED: show expiresAt, hide edit controls, allow viewing own document link */}
                  <Show when={c().status === 'VERIFIED'}>
                    <section class="xc-verification-card" data-case-state="VERIFIED">
                      <h2 class="xc-verification-card-title">{t('verifiedTitle')}</h2>
                      <p class="xc-verification-card-desc">{t('verifiedDesc')}</p>
                      <Show when={c().expiresAt}>
                        {(exp) => (
                          <p class="xc-verification-card-desc" data-expires-at={formatExpiresAt(exp())}>
                            {t('expiresAtLabel', { date: formatExpiresAt(exp()) })}
                          </p>
                        )}
                      </Show>
                      <Show when={c().documents.length > 0}>
                        <ul class="xc-verification-doc-list">
                          <For each={c().documents}>
                            {(doc) => (
                              <li class="xc-verification-doc-item" data-doc-id={doc.id}>
                                <div class="xc-verification-doc-meta">
                                  <span>{doc.documentType}</span>
                                  <span>{doc.mime}</span>
                                </div>
                                <div class="xc-verification-actions">
                                  <button
                                    type="button"
                                    class="xc-verification-btn"
                                    data-action="view-document"
                                    data-view-doc={doc.id}
                                    onClick={() => void handleViewDocument(doc.id)}
                                  >
                                    {t('viewDocButton')}
                                  </button>
                                  <Show when={docLinks()[doc.id]}>
                                    {(link) => (
                                      <a
                                        href={link().url}
                                        target="_blank"
                                        rel="noopener noreferrer"
                                      >
                                        {t('openDocLink')}
                                      </a>
                                    )}
                                  </Show>
                                </div>
                              </li>
                            )}
                          </For>
                        </ul>
                      </Show>
                    </section>
                  </Show>

                  {/* REJECTED, EXPIRED, or REVOKED: show server reason & expiresAt, offer update-and-resubmit (NEW case) */}
                  <Show
                    when={
                      c().status === 'REJECTED' ||
                      c().status === 'EXPIRED' ||
                      c().status === 'REVOKED'
                    }
                  >
                    <section class="xc-verification-card" data-case-state={c().status}>
                      <h2 class="xc-verification-card-title">
                        {c().status === 'EXPIRED'
                          ? t('expiredTitle')
                          : c().status === 'REVOKED'
                            ? t('revokedTitle')
                            : t('rejectedTitle')}
                      </h2>
                      <p class="xc-verification-card-desc">
                        {c().status === 'EXPIRED'
                          ? t('expiredDesc')
                          : c().status === 'REVOKED'
                            ? t('revokedDesc')
                            : t('rejectedDesc')}
                      </p>

                      <Show when={c().decisionReasonCode}>
                        {(reason) => (
                          <div class="xc-verification-reason" role="note">
                            <span class="xc-verification-reason-label">{t('reasonLabel')}</span>
                            <p class="xc-verification-reason-text" data-decision-reason="true">
                              {reason()}
                            </p>
                          </div>
                        )}
                      </Show>

                      <Show when={c().expiresAt}>
                        {(exp) => (
                          <p class="xc-verification-card-desc" data-expires-at={formatExpiresAt(exp())}>
                            {t('expiresAtLabel', { date: formatExpiresAt(exp()) })}
                          </p>
                        )}
                      </Show>

                      <div class="xc-verification-actions">
                        <button
                          type="button"
                          class="xc-verification-btn"
                          data-action="update-and-resubmit"
                          disabled={busyAction()}
                          onClick={() => void handleOpenCase()}
                        >
                          {t('updateAndResubmitButton')}
                        </button>
                      </div>
                    </section>
                  </Show>
                </>
              )}
            </Show>
          </Show>
        </div>
      </Show>
    </>
  );
}
