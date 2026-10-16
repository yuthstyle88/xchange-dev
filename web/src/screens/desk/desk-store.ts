// DeskStore: the state every desk step shares (design xchange-v3-wave3-desk.md §5.3, D-4, D-5).
// Money is held as strings, exactly as typed or as the server answered; the store does no arithmetic (W7).
import { createSignal, onCleanup, type Accessor } from 'solid-js';
import { ApiError, type ApiClient } from '../../shared/api/client';
import type { Me } from '../../shared/api/generated/common';
import type { DeskConfig } from '../../shared/api/generated/desk_config';
import type { QuoteDto } from '../../shared/api/generated/quote';
import type { QuotesCommandInput } from '../../shared/api/generated/quotes';
import { deskPathFor, type DeskPath } from './desk-path';
import { sharedApi, sharedSession } from '../../shared/session/session';
import { screenT } from '../../shared/i18n';
import { wordedError } from '../../shared/i18n/worded-error';
import * as screenI18n from './i18n';
import { createSaver, newKey, type SaveAnswer } from './save';

export type { SaveAnswer } from './save';

export type DeskType = 'BUY' | 'SELL';
export type SaveOutcome = 'idle' | 'saving' | 'saved' | 'refused' | 'unknown';
/** Blueprint §06, verbatim. */
export type TransactionState = 'DRAFT' | 'PENDING_APPROVAL' | 'CONFIRMED' | 'COMPLETED' | 'CANCELLED';
export type StepIndex = 0 | 1 | 2 | 3 | 4;
/** The Blueprint quote (`POST /api/quotes`), as the server answered it. */
export type BlueprintQuote = QuoteDto;

export interface PassportFields {
  name: string;
  passportNo: string;
  nationality: string;
  birthDate: string;
  passportExpiry: string;
}

export interface DeskEntryState {
  kind: 'booking' | 'kiosk';
  id: string;
  banner: Accessor<string>;
  locked: boolean;
  identityLabel?: string;
  identityConfirmed: Accessor<boolean>;
  setIdentityConfirmed(v: boolean): void;
  payloadExtras(): Record<string, string | boolean>;
  beforeReserve?(store: DeskStore): Promise<void>;
  afterSave?(store: DeskStore, answer: SaveAnswer): Promise<void>;
  refusal(): string | undefined;
}

export interface DeskStore {
  /** Steps reach the server through this client (lane controller's ruling 1 on #539). */
  api: ApiClient;
  path: DeskPath;
  branch: Accessor<string>;
  config: Accessor<DeskConfig | undefined>;
  configError: Accessor<string | undefined>;
  step: Accessor<StepIndex>;
  go(step: StepIndex): void;

  // step 0
  type: Accessor<DeskType>;
  setType(t: DeskType): void;
  currency: Accessor<string>;
  setCurrency(c: string): void;
  amount: Accessor<string>;
  setAmount(a: string): void;
  referenceRate: Accessor<string>;
  setReferenceRate(r: string): void;
  quote: Accessor<BlueprintQuote | undefined>;
  quoteError: Accessor<{ field?: string; message: string } | undefined>;
  quoting: Accessor<boolean>;
  requote(): Promise<BlueprintQuote | undefined>;
  // step 1
  passport: Accessor<File | undefined>;
  setPassport(f: File | undefined): void;
  fields: Accessor<PassportFields>;
  setField(k: keyof PassportFields, v: string, by: 'user' | 'ocr'): void;
  editedByUser: Accessor<ReadonlySet<keyof PassportFields>>;
  identityChecked: Accessor<boolean>;
  setIdentityChecked(v: boolean): void;
  customerId: Accessor<string>;
  setCustomerId(id: string): void;
  staffDraftId: Accessor<string>;
  setStaffDraftId(id: string): void;
  // step 2
  staff: Accessor<string>;
  valueDate: Accessor<string>;
  setValueDate(d: string): void;
  booking: Accessor<string>;
  setBooking(ref: string): void;
  // step 3
  method: Accessor<string>;
  setMethod(m: string): void;
  ref: Accessor<string>;
  setRef(r: string): void;
  account: Accessor<string>;
  setAccount(a: string): void;
  fee: Accessor<string>;
  setFee(f: string): void;
  bankFee: Accessor<string>;
  setBankFee(f: string): void;
  notes: Accessor<string>;
  setNotes(n: string): void;
  slip: Accessor<File | undefined>;
  setSlip(f: File | undefined): void;
  paymentChecked: Accessor<boolean>;
  setPaymentChecked(v: boolean): void;
  /** The cash was counted before handing over (desk-devices.js:4); set by the payment step. */
  countConfirmed: Accessor<boolean>;
  setCountConfirmed(v: boolean): void;
  // entries
  entry: Accessor<DeskEntryState | undefined>;
  setEntry(e: DeskEntryState | undefined): void;
  // save (§5.4)
  outcome: Accessor<SaveOutcome>;
  transactionState: Accessor<TransactionState | undefined>;
  saveError: Accessor<string | undefined>;
  saved: Accessor<SaveAnswer | undefined>;
  submit(): Promise<SaveOutcome>;
  backToEdit(): void;
  startNew(): boolean;
}

const QUOTE_DEBOUNCE_MS = 300;

const EMPTY_FIELDS: PassportFields = {
  name: '',
  passportNo: '',
  nationality: '',
  birthDate: '',
  passportExpiry: '',
};

/** Bangkok's today as YYYY-MM-DD (dh:98). */
function bangkokToday(): string {
  return new Intl.DateTimeFormat('en-CA', { timeZone: 'Asia/Bangkok' }).format(new Date());
}

/** The owner's desk names its branch in `?branch=`; staff work in their own branch. */
function pageBranch(path: DeskPath, me: Me | undefined): string {
  if (path.kind === 'owner' && typeof location !== 'undefined') {
    const fromUrl = new URLSearchParams(location.search).get('branch');
    if (fromUrl) return fromUrl;
  }
  return me?.branch ?? '';
}

function quoteErrorOf(e: unknown, fallback: string): { field?: string; message: string } {
  if (e instanceof ApiError) {
    return e.field ? { field: e.field, message: e.message } : { message: e.message };
  }
  return { message: fallback };
}

/** The page's store. Every dependency defaults to the page's own: the shared client and session, and the path of the
 *  page's URL. Tests pass their own (#539 ruling issuecomment-6106545095: main.tsx imports no API client). */
export function createDeskStore(
  given: { api?: ApiClient; path?: DeskPath; me?: Accessor<Me | undefined> } = {},
): DeskStore {
  const api = given.api ?? sharedApi();
  const path = given.path ?? deskPathFor(typeof location !== 'undefined' ? location.pathname : '');
  const deps = { me: given.me ?? sharedSession().me };
  const t = screenT('desk', screenI18n);
  const branch = () => pageBranch(path, deps.me());

  const [config, setConfig] = createSignal<DeskConfig | undefined>();
  const [configError, setConfigError] = createSignal<string | undefined>();
  const [step, setStep] = createSignal<StepIndex>(0);

  const [type, setTypeS] = createSignal<DeskType>('BUY');
  const [currency, setCurrencyS] = createSignal('');
  const [amount, setAmountS] = createSignal('');
  const [referenceRate, setReferenceRate] = createSignal('');
  const [quote, setQuote] = createSignal<BlueprintQuote | undefined>();
  const [quoteError, setQuoteError] = createSignal<{ field?: string; message: string } | undefined>();
  const [quoting, setQuoting] = createSignal(false);

  const [passport, setPassport] = createSignal<File | undefined>();
  const [fields, setFields] = createSignal<PassportFields>({ ...EMPTY_FIELDS });
  const [editedByUser, setEditedByUser] = createSignal<ReadonlySet<keyof PassportFields>>(new Set());
  const [identityChecked, setIdentityChecked] = createSignal(false);
  const [customerId, setCustomerId] = createSignal('');
  const [staffDraftId, setStaffDraftId] = createSignal('');

  const [valueDate, setValueDate] = createSignal(bangkokToday());
  const [booking, setBooking] = createSignal('');

  const [method, setMethod] = createSignal('');
  const [ref, setRef] = createSignal('');
  const [account, setAccount] = createSignal('');
  const [fee, setFeeS] = createSignal('');
  const [bankFee, setBankFee] = createSignal('');
  const [notes, setNotes] = createSignal('');
  const [slip, setSlip] = createSignal<File | undefined>();
  const [paymentChecked, setPaymentChecked] = createSignal(false);
  const [countConfirmed, setCountConfirmed] = createSignal(false);

  const [entry, setEntry] = createSignal<DeskEntryState | undefined>();

  const [outcome, setOutcome] = createSignal<SaveOutcome>('idle');
  const [transactionState, setTransactionState] = createSignal<TransactionState | undefined>();
  const [saveError, setSaveError] = createSignal<string | undefined>();
  const [saved, setSaved] = createSignal<SaveAnswer | undefined>();

  // ── the config view: read once, on create (§3.1 L3, no polling) ──
  const b = branch();
  api
    .view<DeskConfig>(path.configView, b ? { branch: b } : {})
    .then((answer) => {
      const c = answer.data;
      setConfig(c);
      if (c && currency() === '' && c.rates.length > 0) {
        setCurrencyS(c.rates[0].code);
      }
    })
    .catch((e: unknown) => {
      setConfigError(wordedError(e));
    });

  // ── the quote (D-4, D-5) ──
  let quoteSeq = 0;
  let timer: ReturnType<typeof setTimeout> | undefined;

  function cancelScheduled(): void {
    if (timer !== undefined) {
      clearTimeout(timer);
      timer = undefined;
    }
  }

  function scheduleQuote(): void {
    cancelScheduled();
    timer = setTimeout(() => {
      timer = undefined;
      void requote();
    }, QUOTE_DEBOUNCE_MS);
  }

  async function requote(): Promise<BlueprintQuote | undefined> {
    cancelScheduled();
    if (currency() === '' || amount() === '') {
      return undefined;
    }
    const mine = ++quoteSeq;
    setQuoting(true);
    const feeThb = fee();
    const body: QuotesCommandInput = {
      branch: null,
      branchId: null,
      customerId: null,
      channel: 'DESK',
      lines: [
        {
          currency: currency(),
          shopSide: type(),
          quantity: amount(),
          feeThb: feeThb === '' ? null : feeThb,
          denominationGroup: null,
        },
      ],
    };
    try {
      const answered = await api.command<QuotesCommandInput, QuoteDto>(
        'POST',
        path.quoteRoutes.quotes,
        body,
        newKey(),
      );
      if (mine !== quoteSeq) return quote();
      setQuote(answered);
      setQuoteError(undefined);
      return answered;
    } catch (e) {
      if (mine !== quoteSeq) return quote();
      setQuote(undefined);
      setQuoteError(quoteErrorOf(e, t('quoteFailed')));
      return undefined;
    } finally {
      if (mine === quoteSeq) setQuoting(false);
    }
  }

  onCleanup(() => {
    cancelScheduled();
    quoteSeq += 1;
  });

  function setType(v: DeskType): void {
    setTypeS(v);
    scheduleQuote();
  }
  function setCurrency(v: string): void {
    setCurrencyS(v);
    scheduleQuote();
  }
  function setAmount(v: string): void {
    setAmountS(v);
    scheduleQuote();
  }
  function setFee(v: string): void {
    setFeeS(v);
    setPaymentChecked(false);
    scheduleQuote();
  }

  function setField(k: keyof PassportFields, v: string, by: 'user' | 'ocr'): void {
    setFields({ ...fields(), [k]: v });
    if (by === 'user') {
      const next = new Set(editedByUser());
      next.add(k);
      setEditedByUser(next);
      setIdentityChecked(false);
    }
  }

  const store: DeskStore = {
    api,
    path,
    branch,
    config,
    configError,
    step,
    go: (s) => setStep(s),
    type,
    setType,
    currency,
    setCurrency,
    amount,
    setAmount,
    referenceRate,
    setReferenceRate,
    quote,
    quoteError,
    quoting,
    requote,
    passport,
    setPassport: (f) => setPassport(f),
    fields,
    setField,
    editedByUser,
    identityChecked,
    setIdentityChecked,
    customerId,
    setCustomerId,
    staffDraftId,
    setStaffDraftId,
    staff: () => deps.me()?.name ?? deps.me()?.id ?? '',
    valueDate,
    setValueDate,
    booking,
    setBooking,
    method,
    setMethod,
    ref,
    setRef,
    account,
    setAccount,
    fee,
    setFee,
    bankFee,
    setBankFee,
    notes,
    setNotes,
    slip,
    setSlip: (f) => setSlip(f),
    paymentChecked,
    setPaymentChecked,
    countConfirmed,
    setCountConfirmed,
    entry,
    setEntry: (e) => setEntry(e),
    outcome,
    transactionState,
    saveError,
    saved,
    submit: () => saver.submit(),
    backToEdit,
    startNew,
  };

  const saver = createSaver(store, {
    setOutcome,
    setTransactionState,
    setSaveError,
    setSaved,
  });
  onCleanup(() => saver.dispose());

  /** DK-1: drop the booking, the submission and its keys, the quote and staffDraftId; back to step 0. */
  function backToEdit(): void {
    saver.reset();
    setBooking('');
    setQuote(undefined);
    setStaffDraftId('');
    setOutcome('idle');
    setTransactionState(undefined);
    setSaveError(undefined);
    setSaved(undefined);
    setStep(0);
  }

  /** v2's New (dh:118): asks while unsaved; keeps `staff` and `valueDate`; clears staffDraftId (DK-1). */
  function startNew(): boolean {
    const unsaved =
      outcome() !== 'saved' &&
      (amount() !== '' || customerId() !== '' || passport() !== undefined || transactionState() !== undefined);
    if (unsaved && !window.confirm(t('newUnsaved'))) {
      return false;
    }
    cancelScheduled();
    quoteSeq += 1;
    saver.reset();
    setTypeS('BUY');
    setAmountS('');
    setReferenceRate('');
    setQuote(undefined);
    setQuoteError(undefined);
    setQuoting(false);
    setPassport(undefined);
    setFields({ ...EMPTY_FIELDS });
    setEditedByUser(new Set<keyof PassportFields>());
    setIdentityChecked(false);
    setCustomerId('');
    setStaffDraftId('');
    setBooking('');
    setMethod('');
    setRef('');
    setAccount('');
    setFeeS('');
    setBankFee('');
    setNotes('');
    setSlip(undefined);
    setPaymentChecked(false);
    setCountConfirmed(false);
    setEntry(undefined);
    setOutcome('idle');
    setTransactionState(undefined);
    setSaveError(undefined);
    setSaved(undefined);
    setStep(0);
    return true;
  }

  return store;
}
