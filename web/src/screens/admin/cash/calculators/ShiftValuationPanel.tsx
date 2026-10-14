import { createEffect, createSignal, For, Show, type JSX } from 'solid-js';
import { ApiError, type ApiClient } from '../../../../shared/api/client';
import type {
  ShiftValuation,
  ShiftValuationCurrency,
  ShiftValuationRequest,
} from '../../../../shared/api/generated/shift_valuation';
import { locale, screenT } from '../../../../shared/i18n';
import { wordedError } from '../../../../shared/i18n/worded-error';
import { MoneyText, formatMoney } from '../../../../shared/ui/MoneyText';
import * as table from './i18n';

const t = screenT('admin.cash.calculators', table);

type KindKey = 'kindTransfer' | 'kindNotes' | 'kindSell';
const KINDS: KindKey[] = ['kindTransfer', 'kindNotes', 'kindSell'];

interface Shown {
  answer?: ShiftValuation;
  date?: string;
  kind?: KindKey;
  at?: Date;
  error?: string;
}

export function ShiftValuationPanel(props: { api: ApiClient; branch: string; shiftId: string }): JSX.Element {
  const [currencies, setCurrencies] = createSignal<ShiftValuationCurrency[]>([]);
  const [rates, setRates] = createSignal<Record<string, string>>({});
  const [date, setDate] = createSignal('');
  const [kind, setKind] = createSignal<KindKey>('kindTransfer');
  const [shown, setShown] = createSignal<Shown | null>(null);
  let listSeq = 0;
  let submitSeq = 0;
  let currentBranch = props.branch;
  let currentShiftId = props.shiftId;

  type Body = ShiftValuationRequest & { branch: string };

  async function list(): Promise<boolean> {
    const id = ++listSeq;
    try {
      const a = await props.api.act<Body, ShiftValuation>('compute', 'shift_valuation', {
        branch: props.branch,
        shiftId: props.shiftId,
      });
      if (id !== listSeq) return false;
      setCurrencies(a.currencies);
      return true;
    } catch (e) {
      if (id !== listSeq) return false;
      setCurrencies([]);
      setShown({ error: wordedError(e) });
      return false;
    }
  }

  createEffect(() => {
    const b = props.branch;
    const s = props.shiftId;
    if (b !== currentBranch || s !== currentShiftId) {
      currentBranch = b;
      currentShiftId = s;
      setShown(null);
      setCurrencies([]);
      setRates({});
    }
    void list();
  });

  const changed = (): void => {
    submitSeq += 1;
  };

  const submit = async (e: Event): Promise<void> => {
    e.preventDefault();
    const id = ++submitSeq;
    const sent: Record<string, string> = {};
    for (const c of currencies()) {
      if (c.needsRate) sent[c.currency] = (rates()[c.currency] ?? '').trim();
    }
    const d = date();
    const k = kind();
    try {
      const a = await props.api.act<Body, ShiftValuation>('compute', 'shift_valuation', {
        branch: props.branch,
        shiftId: props.shiftId,
        rates: sent,
      });
      if (id !== submitSeq) return;
      setShown({ answer: a, date: d, kind: k, at: new Date() });
    } catch (err) {
      if (id !== submitSeq) return;
      if (err instanceof ApiError && err.code === 'valuationRate') {
        await list();
        if (id !== submitSeq) return;
      }
      setShown({ error: wordedError(err) });
    }
  };

  return (
    <section class="calc-panel shift-valuation">
      <h3>{t('valuationHeading')}</h3>
      <p>{t('valuationExplain')}</p>
      <p class="dialog-note">{t('valuationNote')}</p>
      <p>
        <a class="text-link" href="https://www.bot.or.th/th/statistics/exchange-rate.html" target="_blank" rel="noopener noreferrer">
          {t('botLink')}
        </a>
      </p>
      <form onSubmit={submit}>
        <div class="form-grid">
          <label>
            {t('rateDate')}
            <input
              type="date"
              required
              value={date()}
              onInput={(e) => {
                setDate(e.currentTarget.value);
                changed();
              }}
            />
          </label>
          <label>
            {t('rateKind')}
            <select
              value={kind()}
              onChange={(e) => {
                setKind(e.currentTarget.value as KindKey);
                changed();
              }}
            >
              <For each={KINDS}>{(k) => <option value={k}>{t(k)}</option>}</For>
            </select>
          </label>
        </div>
        <p class="footnote">{t('rateFootnote')}</p>
        <For each={currencies().filter((c) => c.needsRate)}>
          {(c) => (
            <label>
              {t('rateLabel', { c: c.currency })}
              <input
                type="number"
                min="0.000001"
                max="1000000"
                step="any"
                required
                placeholder={t('ratePlaceholder')}
                value={rates()[c.currency] ?? ''}
                onInput={(e) => {
                  setRates({ ...rates(), [c.currency]: e.currentTarget.value });
                  changed();
                }}
              />
            </label>
          )}
        </For>
        <button class="primary">{t('valuationSubmit')}</button>
      </form>
      <div aria-live="polite">
        <Show when={shown()}>
          {(s) => (
            <Show when={s().answer?.result} fallback={s().error}>
              {(result) => (
                <>
                  <p>
                    {t('resultLine', { date: s().date ?? '', kind: t(s().kind ?? 'kindTransfer') })}
                    <br />
                    {t('computedAt', { when: (s().at ?? new Date()).toLocaleString(locale()) })}
                  </p>
                  <table>
                    <thead>
                      <tr>
                        <th>{t('colCurrency')}</th>
                        <th class="num">{t('colOpening')}</th>
                        <th class="num">{t('colCurrent')}</th>
                        <th class="num">{t('colRate')}</th>
                        <th class="num">{t('colValue')}</th>
                      </tr>
                    </thead>
                    <tbody>
                      <For each={s().answer!.currencies}>
                        {(c, i) => (
                          <tr>
                            <td>{c.currency}</td>
                            <td class="num">{formatMoney(c.opening)}</td>
                            <td class="num">{formatMoney(c.current)}</td>
                            <td class="num">{result().rows[i()]?.rate ?? ''}</td>
                            <td class="num">{formatMoney(result().rows[i()]?.value ?? '')}</td>
                          </tr>
                        )}
                      </For>
                    </tbody>
                  </table>
                  <dl class="details">
                    <div>
                      <dt>{t('totalCurrent')}</dt>
                      <dd>
                        <MoneyText value={result().current} currency="THB" />
                      </dd>
                    </div>
                    <div>
                      <dt>{t('totalOpening')}</dt>
                      <dd>
                        <MoneyText value={result().opening} currency="THB" />
                      </dd>
                    </div>
                    <div>
                      <dt>{t('diffBefore')}</dt>
                      <dd>
                        <MoneyText value={result().difference} currency="THB" />
                      </dd>
                    </div>
                    <div>
                      <dt>{t('movementsNet')}</dt>
                      <dd>
                        <MoneyText value={result().movements} currency="THB" />
                      </dd>
                    </div>
                    <div>
                      <dt>{t('diffAfter')}</dt>
                      <dd>
                        <strong>
                          <MoneyText value={result().adjusted} currency="THB" />
                        </strong>
                      </dd>
                    </div>
                  </dl>
                  <p class="dialog-note">{t('closingNote')}</p>
                </>
              )}
            </Show>
          )}
        </Show>
      </div>
    </section>
  );
}
