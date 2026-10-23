// Step 1, passport / customer (design xchange-v3-wave3-desk.md §3.2 rows 1 and "1 skip", §3.9 DK-1; #539 Part 3).
// Calls go through `store.api` on `store.path.rpc` (lane controller's ruling 1 on #539). Nationality is an ISO
// alpha-3 picker, never free text (CI-4 ruling (a)); the skip sends `XXX` (owner 2026-10-11). No arithmetic (W7).
import { createSignal, For, onCleanup, Show, type Component, type JSX } from 'solid-js';
import { Dynamic } from 'solid-js/web';
import type { DraftBeginInput } from '../../../../shared/api/generated/branch_day';
import { keyedWrite } from '../../../../shared/api/keyed-write';
import { screenT } from '../../../../shared/i18n';
import { wordedError } from '../../../../shared/i18n/worded-error';
import type { DeskStore, PassportFields } from '../../desk-store';
import type { DeskStep } from '../../steps';
import * as texts from './i18n';
import { NATIONALITIES, UNSPECIFIED_NATIONALITY } from './nationalities';

const t = screenT('desk.steps.customer', texts);

/** 15 MB, v2's limit (dh:104). */
const MAX_FILE_BYTES = 15_728_640;
const ACCEPTED_TYPES = ['image/jpeg', 'image/png', 'image/webp', 'application/pdf'];
const ACCEPT = '.jpg,.jpeg,.png,.webp,.pdf,image/jpeg,image/png,image/webp,application/pdf';
const MAX_NAME_LEN = 100;
const MAX_PASSPORT_NO_LEN = 30;
const MIN_DATE = '1920-04-02';

/** The optional passport reader (DK-F, design §6): mounted when `ocr/PassportOcr.tsx` exists, skipped otherwise. */
const ocrModules = import.meta.glob<{ PassportOcr?: Component<{ store: DeskStore }> }>('../../ocr/PassportOcr.tsx', {
  eager: true,
});
const PassportOcr = Object.values(ocrModules)[0]?.PassportOcr;

/** v2's refusal of a file it cannot take (dh:104), or undefined when the file is kept. */
function fileRefusal(file: File): string | undefined {
  const name = file.name.toLowerCase();
  const heic = file.type === 'image/heic' || file.type === 'image/heif' || /\.(heic|heif)$/.test(name);
  const known = ACCEPTED_TYPES.includes(file.type) || /\.(jpe?g|png|webp|pdf)$/.test(name);
  if (heic || !known) return t('heic');
  if (file.size === 0 || file.size > MAX_FILE_BYTES) return t('tooBig');
  return undefined;
}

/** A PNG that reads `ไม่มีเอกสารพาสปอร์ต` (desk-skip.js:5). Without a canvas it is a 1×1 PNG of that name. */
async function noPassportImage(): Promise<File> {
  const canvas = document.createElement('canvas');
  canvas.width = 600;
  canvas.height = 240;
  let ctx: CanvasRenderingContext2D | null = null;
  try {
    ctx = canvas.getContext('2d');
  } catch {
    ctx = null;
  }
  if (ctx && typeof canvas.toBlob === 'function') {
    ctx.fillStyle = '#ffffff';
    ctx.fillRect(0, 0, canvas.width, canvas.height);
    ctx.fillStyle = '#000000';
    ctx.font = '32px sans-serif';
    ctx.textAlign = 'center';
    ctx.textBaseline = 'middle';
    ctx.fillText(texts.th.noPassportImage, canvas.width / 2, canvas.height / 2);
    const blob = await new Promise<Blob | null>((resolve) => canvas.toBlob(resolve, 'image/png'));
    if (blob) return new File([blob], 'no-passport.png', { type: 'image/png' });
  }
  const bytes = Uint8Array.from(
    atob('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8/x8AAwMCAO+ip1sAAAAASUVORK5CYII='),
    (c) => c.charCodeAt(0),
  );
  return new File([bytes], 'no-passport.png', { type: 'image/png' });
}

function CustomerStep(props: { store: DeskStore }): JSX.Element {
  const store = props.store;
  const draftWrite = keyedWrite<DraftBeginInput, { id?: unknown }>(store.api, 'POST', '/api/tx-drafts');
  const [error, setError] = createSignal<string | undefined>();
  const [fileError, setFileError] = createSignal<string | undefined>();
  const [fieldErrors, setFieldErrors] = createSignal<Partial<Record<keyof PassportFields, string>>>({});
  const [sending, setSending] = createSignal(false);
  let fileEl: HTMLInputElement | undefined;
  let alive = true;
  onCleanup(() => {
    alive = false;
  });

  const field = (k: keyof PassportFields) => store.fields()[k];
  const edit = (k: keyof PassportFields) => (e: { currentTarget: HTMLInputElement | HTMLSelectElement }) => {
    setFieldErrors((prev) => ({ ...prev, [k]: undefined }));
    const el = e.currentTarget;
    const val = el instanceof HTMLInputElement && el.maxLength > 0 ? el.value.slice(0, el.maxLength) : el.value;
    if (val !== el.value) el.value = val;
    store.setField(k, val, 'user');
  };

  function onFile(e: Event & { currentTarget: HTMLInputElement }): void {
    const file = e.currentTarget.files?.[0];
    setFileError(undefined);
    if (!file) return;
    const refusal = fileRefusal(file);
    if (refusal) {
      setFileError(refusal);
      e.currentTarget.value = '';
      return;
    }
    store.setPassport(file);
  }

  /** draftBegin for staff (X5, #3008), the entry's own step, then reserveCustomer; step 2 on success. */
  async function reserve(prefix?: (reason: string) => string): Promise<void> {
    setSending(true);
    setError(undefined);
    try {
      if (store.path.usesDraft && store.staffDraftId() === '') {
        const draft = await draftWrite.send({
          branch: store.branch(),
          side: store.type(),
          currency: store.currency(),
          amount: store.amount(),
        });
        if (!alive) return;
        if (typeof draft.id === 'string' && draft.id !== '') store.setStaffDraftId(draft.id);
      }
      await store.entry()?.beforeReserve?.(store);
      if (!alive) return;
      const f = store.fields();
      const answer = await store.api.post<
        { action: string; payload: PassportFields & { mode: 'LIVE' } },
        { customerId?: unknown; existing?: unknown }
      >(store.path.rpc, {
        action: 'reserveCustomer',
        payload: {
          name: f.name,
          passportNo: f.passportNo.toUpperCase(),
          nationality: f.nationality,
          birthDate: f.birthDate,
          passportExpiry: f.passportExpiry,
          mode: 'LIVE',
        },
      });
      if (!alive) return;
      if (typeof answer.customerId !== 'string' || answer.customerId === '') {
        setError(prefix ? prefix(t('noServer')) : t('noServer'));
        return;
      }
      store.setCustomerId(answer.customerId);
      store.go(2);
    } catch (e) {
      if (!alive) return;
      const reason = wordedError(e);
      setError(prefix ? prefix(reason) : reason);
    } finally {
      if (alive) setSending(false);
    }
  }

  function onSubmit(ev: Event): void {
    ev.preventDefault();
    if (sending()) return;
    setError(undefined);
    setFieldErrors({});
    if (!store.passport()) {
      setError(t('noFile'));
      return;
    }
    const name = field('name').trim();
    const passportNo = field('passportNo').trim();
    if (name === '' || passportNo === '' || field('nationality') === '') {
      setError(t('missing'));
      return;
    }
    const nextFieldErrors: Partial<Record<keyof PassportFields, string>> = {};
    if (name.length > MAX_NAME_LEN) nextFieldErrors.name = t('nameTooLong');
    if (passportNo.length > MAX_PASSPORT_NO_LEN) nextFieldErrors.passportNo = t('passportNoTooLong');
    const birthDate = field('birthDate').trim();
    if (birthDate !== '' && birthDate < MIN_DATE) nextFieldErrors.birthDate = t('dateTooEarly');
    const passportExpiry = field('passportExpiry').trim();
    if (passportExpiry !== '' && passportExpiry < MIN_DATE) nextFieldErrors.passportExpiry = t('dateTooEarly');
    if (Object.keys(nextFieldErrors).length > 0) {
      setFieldErrors(nextFieldErrors);
      return;
    }
    void reserve();
  }

  async function onSkip(): Promise<void> {
    if (sending()) return;
    setSending(true);
    setError(undefined);
    try {
      const img = await noPassportImage();
      if (!alive) return;
      store.setPassport(img);
    } catch (e) {
      if (!alive) return;
      setError(t('skipFailed', { reason: wordedError(e) }));
      setSending(false);
      return;
    }
    store.setField('name', 'WALK-IN', 'user');
    store.setField('passportNo', 'WALKIN', 'user');
    store.setField('nationality', UNSPECIFIED_NATIONALITY, 'user');
    await reserve((reason) => t('skipFailed', { reason }));
  }

  return (
    <form class="desk-form" onSubmit={onSubmit}>
      <h2>{t('heading')}</h2>

      <label class="desk-field">
        <span>{t('file')}</span>
        <input ref={fileEl} type="file" name="passport" accept={ACCEPT} onChange={onFile} />
        <Show when={fileError()}>
          <span class="desk-error">{fileError()}</span>
        </Show>
        <Show when={store.passport()}>
          {(file) => (
            <span class="desk-file">
              {file().name}{' '}
              <button
                type="button"
                onClick={() => {
                  store.setPassport(undefined);
                  if (fileEl) fileEl.value = '';
                }}
              >
                {t('removeFile')}
              </button>
            </span>
          )}
        </Show>
      </label>

      <Show when={PassportOcr}>{(ocr) => <Dynamic component={ocr()} store={store} />}</Show>

      <label class="desk-field">
        <span>{t('name')}</span>
        <input name="name" autocomplete="off" maxLength={MAX_NAME_LEN} value={field('name')} onInput={edit('name')} />
        <Show when={fieldErrors().name}>
          <span class="desk-error" role="alert">
            {fieldErrors().name}
          </span>
        </Show>
      </label>
      <label class="desk-field">
        <span>{t('passportNo')}</span>
        <input
          name="passportNo"
          autocomplete="off"
          maxLength={MAX_PASSPORT_NO_LEN}
          value={field('passportNo')}
          onInput={edit('passportNo')}
        />
        <Show when={fieldErrors().passportNo}>
          <span class="desk-error" role="alert">
            {fieldErrors().passportNo}
          </span>
        </Show>
      </label>
      <label class="desk-field">
        <span>{t('nationality')}</span>
        <select name="nationality" value={field('nationality')} onChange={edit('nationality')}>
          <option value="">{t('nationalityPick')}</option>
          <For each={NATIONALITIES}>{(code) => <option value={code}>{code}</option>}</For>
        </select>
      </label>
      <label class="desk-field">
        <span>{t('birthDate')}</span>
        <input name="birthDate" type="date" min={MIN_DATE} value={field('birthDate')} onInput={edit('birthDate')} />
        <Show when={fieldErrors().birthDate}>
          <span class="desk-error" role="alert">
            {fieldErrors().birthDate}
          </span>
        </Show>
      </label>
      <label class="desk-field">
        <span>{t('passportExpiry')}</span>
        <input
          name="passportExpiry"
          type="date"
          min={MIN_DATE}
          value={field('passportExpiry')}
          onInput={edit('passportExpiry')}
        />
        <Show when={fieldErrors().passportExpiry}>
          <span class="desk-error" role="alert">
            {fieldErrors().passportExpiry}
          </span>
        </Show>
      </label>

      <label class="desk-check">
        <input
          type="checkbox"
          name="identityChecked"
          checked={store.identityChecked()}
          onChange={(e) => store.setIdentityChecked(e.currentTarget.checked)}
        />
        <span>{t('identityChecked')}</span>
      </label>

      <Show when={error()}>
        <p class="desk-error" role="alert">
          {error()}
        </p>
      </Show>
      <Show when={sending()}>
        <p class="desk-notice">{t('sending')}</p>
      </Show>

      <div class="desk-actions">
        <button type="button" disabled={sending()} onClick={() => store.go(0)}>
          {t('back')}
        </button>
        <button type="button" disabled={sending()} onClick={() => void onSkip()}>
          {t('skip')}
        </button>
        <button type="submit" disabled={sending()}>
          {store.customerId() ? t('submitKnown') : t('submitNew')}
        </button>
      </div>
    </form>
  );
}

export const step: DeskStep<DeskStore> = {
  index: 1,
  label: () => t('label'),
  component: (p) => <CustomerStep store={p.store} />,
};
