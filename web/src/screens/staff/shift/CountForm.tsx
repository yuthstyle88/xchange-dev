import { createEffect, createSignal, onCleanup, onMount, type Accessor, type JSX } from 'solid-js';
import { ApiClient } from '../../../shared/api/client';
import type { CashLineInput, CountSubmitInput } from '../../../shared/api/generated/cash';
import type { CashCountRow } from '../../../shared/api/generated/cash_counts';
import type { DenominationRow } from '../../../shared/api/generated/denominations';
import type { MyShift } from '../../../shared/api/generated/my_shift';
import { screenT } from '../../../shared/i18n';
import { fieldError } from '../../../shared/i18n/field-error';
import { wordedError } from '../../../shared/i18n/worded-error';
import { checkInteger } from '../../../shared/ui/bounded';
import { CurrencyCount } from '../../../shared/ui/currency-count/CurrencyCount';
import { yieldQueueCard } from '../../../shared/ui/queue-card-yield';
import type { StaffCtx } from '../cards';
import { command } from '../parts/command';
import { formatCountTime } from './counts';
import { en, th } from './i18n';

export interface CountFormProps {
  ctx: StaffCtx;
  kind: 'OPEN' | 'CLOSE';
  denominations?: Accessor<DenominationRow[]>;
  busy: Accessor<boolean>;
  setBusy(v: boolean): void;
  onSaved(): Promise<void>;
  onCancel(): void;
  onError(msg: string | null): void;
}

export function CountForm(p: CountFormProps): JSX.Element {
  const t = screenT('staff.shift', { th, en });
  const api = new ApiClient();
  const submitCmd = command<CountSubmitInput, { id: string }>(api, 'POST', '/api/cash-counts');

  const [ownDenominations, setOwnDenominations] = createSignal<DenominationRow[]>([]);
  const [myShift, setMyShift] = createSignal<MyShift | null>(null);
  const [myShiftLoaded, setMyShiftLoaded] = createSignal<boolean>(p.kind !== 'CLOSE');
  const [quantities, setQuantities] = createSignal<Record<number, string>>({});
  const [lineErrors, setLineErrors] = createSignal<Record<number, string>>({});

  const denoms = (): DenominationRow[] => (p.denominations ? p.denominations() : ownDenominations());

  createEffect(() => {
    const release = yieldQueueCard();
    onCleanup(release);
  });

  onMount(() => {
    if (!p.denominations) {
      void (async () => {
        try {
          const res = await api.view<DenominationRow[] | { rows: DenominationRow[] }>('denominations');
          const data = res.data;
          if (Array.isArray(data)) {
            setOwnDenominations(data);
          } else if (data && Array.isArray((data as { rows: DenominationRow[] }).rows)) {
            setOwnDenominations((data as { rows: DenominationRow[] }).rows);
          }
        } catch (err: unknown) {
          p.onError(wordedError(err));
        }
      })();
    }
    if (p.kind === 'CLOSE') {
      void (async () => {
        try {
          const res = await api.view<MyShift | null>('my_shift');
          setMyShift(res.data ?? null);
        } catch {
          setMyShift(null);
        } finally {
          setMyShiftLoaded(true);
        }
      })();
    }
  });

  const countLocked = (): string[] => {
    const codes = ['THB'];
    if (p.kind === 'CLOSE') {
      for (const b of myShift()?.balances ?? []) {
        if (/[1-9]/.test(b.expected) && !codes.includes(b.code)) {
          codes.push(b.code);
        }
      }
    }
    return codes;
  };

  const countExpected = (): Record<string, string> | undefined => {
    if (p.kind !== 'CLOSE') return undefined;
    const shift = myShift();
    if (!shift) return undefined;
    const out: Record<string, string> = {};
    for (const b of shift.balances) out[b.code] = b.expected;
    return out;
  };

  const saveDisabled = (): boolean => p.busy() || denoms().length === 0 || !myShiftLoaded();

  const handleSubmit = async (e: Event) => {
    e.preventDefault();
    const form = e.currentTarget as HTMLFormElement;
    if (saveDisabled()) return;

    const inputs = form.querySelectorAll<HTMLInputElement>('input[data-denomination-id]');
    const lines: CashLineInput[] = [];
    const nextQuantities: Record<number, string> = {};
    const nextErrors: Record<number, string> = {};
    const labels: Record<string, string> = {};
    const lineDenomIds: number[] = [];
    const list = denoms();

    inputs.forEach((input) => {
      const idStr = input.getAttribute('data-denomination-id');
      if (!idStr) return;
      const d = list.find((x) => String(x.id) === idStr);
      const id = d ? d.id : +idStr;
      const qty = input.value.trim();
      nextQuantities[id] = input.value;
      if (qty !== '' && d) {
        const check = checkInteger(qty, 0, 2147483647);
        if (!check.ok) {
          nextErrors[d.id] = check.error === 'outOfRange' ? t('qtyTooLarge') : t('qtyWholeMin0');
          return;
        }
        const idx = lines.length;
        labels[`lines[${idx}].qty`] = `${d.currency} ${d.faceValue}`;
        lineDenomIds.push(d.id);
        lines.push({
          currency: d.currency,
          denominationId: d.id,
          qty: check.text,
        });
      }
    });

    setQuantities(nextQuantities);
    if (Object.keys(nextErrors).length > 0) {
      setLineErrors(nextErrors);
      return;
    }
    setLineErrors({});

    const body: CountSubmitInput = {
      branch: p.ctx.me.branchId,
      kind: p.kind,
      shiftId: p.kind === 'CLOSE' ? (p.ctx.floor?.myShift?.id ?? null) : null,
      lines,
      note: null,
    };

    p.setBusy(true);
    try {
      await submitCmd.send(body);
      p.onError(null);
      setQuantities({});
      setLineErrors({});
      await p.onSaved();
    } catch (err: unknown) {
      const fe = fieldError(err, labels);
      const match = fe.field?.match(/^lines\[(\d+)\]\.qty$/);
      const denomId = match ? lineDenomIds[+match[1]!] : undefined;
      if (denomId !== undefined) {
        setLineErrors({ [denomId]: fe.message });
      } else {
        p.onError(fe.message);
      }
    } finally {
      p.setBusy(false);
    }
  };

  return (
    <form data-testid="shift-count-form" class="xc-shift-count-form" onSubmit={handleSubmit}>
      <CurrencyCount
        denominations={denoms()}
        locked={countLocked()}
        expected={countExpected()}
        quantities={quantities()}
        setQuantity={(id, v) => setQuantities((prev) => ({ ...prev, [id]: v }))}
        lineErrors={lineErrors()}
      />
      <div class="xc-shift-form-actions">
        <button
          type="submit"
          data-testid="shift-count-save"
          class="xc-shift-save-btn"
          disabled={saveDisabled()}
        >
          {t('save')}
        </button>
        <button type="button" class="xc-shift-cancel-btn" onClick={() => p.onCancel()}>
          {t('cancel')}
        </button>
      </div>
    </form>
  );
}

export function CountWaitingLine(p: { kind: 'OPEN' | 'CLOSE'; row: CashCountRow }): JSX.Element {
  const t = screenT('staff.shift', { th, en });
  return (
    <div
      role="status"
      class="xc-shift-close-waiting"
      data-testid={p.kind === 'OPEN' ? 'shift-open-waiting' : 'shift-close-waiting'}
    >
      {t(p.kind === 'OPEN' ? 'openWaiting' : 'closeWaiting')} ({formatCountTime(p.row.countedAt)})
    </div>
  );
}
