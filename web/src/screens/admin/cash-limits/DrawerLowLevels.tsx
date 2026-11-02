import {
  createEffect,
  createSignal,
  Index,
  Show,
  type Accessor,
  type JSX,
} from 'solid-js';
import { ApiClient } from '../../../shared/api/client';
import type { CashLimitsData } from '../../../shared/api/generated/cash_limits';
import type { DrawerLowLevelsSaveInput } from '../../../shared/api/generated/drawer_low_levels_save';
import { keyedWrite } from '../../../shared/api/keyed-write';
import { screenT } from '../../../shared/i18n';
import { Card, CardBody, CardHead } from '../../../shared/ui/Card';
import { toast } from '../parts/toast';
import { limitsError } from './CashLimits';
import * as table from './i18n';

const t = screenT('admin.cash-limits', table);

export interface DrawerLowLevelsProps {
  data: Accessor<CashLimitsData | null>;
  branch: Accessor<string>;
  onSaved(data: CashLimitsData): void;
}

interface FormRow {
  currency: string;
  lowLevel: string;
  isNew: boolean;
}

export function DrawerLowLevels(props: DrawerLowLevelsProps): JSX.Element {
  const api = new ApiClient();
  const saveWrite = keyedWrite<
    DrawerLowLevelsSaveInput & { branch: string },
    CashLimitsData
  >(api, 'PUT', '/api/cash-limits/drawer-low-levels');

  const [rows, setRows] = createSignal<FormRow[]>([]);
  const [busy, setBusy] = createSignal(false);
  const [refusal, setRefusal] = createSignal<string | null>(null);

  createEffect(() => {
    props.branch();
    const d = props.data();
    if (!d) {
      setRows([]);
      setRefusal(null);
      return;
    }
    setRows(
      d.drawerLowLevels.map((l) => ({
        currency: l.currency,
        lowLevel: l.lowLevel,
        isNew: false,
      })),
    );
    setRefusal(null);
  });

  async function save(e: Event) {
    e.preventDefault();
    if (busy()) return;
    const body: DrawerLowLevelsSaveInput & { branch: string } = {
      branch: props.branch(),
      levels: rows().map((r) => ({
        currency: r.currency.trim(),
        lowLevel: r.lowLevel.trim(),
      })),
    };
    setBusy(true);
    setRefusal(null);
    try {
      const saved = await saveWrite.send(body);
      props.onSaved(saved);
      toast(t('levelsSaved'));
    } catch (err: unknown) {
      setRefusal(limitsError(err));
    } finally {
      setBusy(false);
    }
  }

  function addRow() {
    setRows((prev) => [...prev, { currency: '', lowLevel: '', isNew: true }]);
  }

  function removeRow(index: number) {
    setRows((prev) => prev.filter((_, i) => i !== index));
  }

  return (
    <Show when={props.data()}>
      <div class="drawer-low-levels" data-section="drawer-low-levels">
        <Card>
          <CardHead title={t('levelsTitle')} />
          <CardBody>
            <p class="levels-help">{t('levelsHelp')}</p>
            <form onSubmit={save}>
              <Show
                when={rows().length > 0}
                fallback={<p class="levels-empty">{t('levelsEmpty')}</p>}
              >
                <table class="levels-table">
                  <thead>
                    <tr>
                      <th>{t('levelCurrency')}</th>
                      <th>{t('levelAmount')}</th>
                      <th></th>
                    </tr>
                  </thead>
                  <tbody>
                    <Index each={rows()}>
                      {(row, index) => (
                        <tr data-level-row>
                          <td>
                            <Show
                              when={row().isNew}
                              fallback={
                                <span class="currency-code" data-field="currency">
                                  {row().currency}
                                </span>
                              }
                            >
                              <input
                                type="text"
                                name="currency"
                                data-field="currency"
                                maxLength={3}
                                value={row().currency}
                                onInput={(e) => {
                                  const upper = e.currentTarget.value
                                    .slice(0, 3)
                                    .toUpperCase();
                                  e.currentTarget.value = upper;
                                  setRows((prev) =>
                                    prev.map((r, i) =>
                                      i === index
                                        ? { ...r, currency: upper }
                                        : r,
                                    ),
                                  );
                                }}
                              />
                            </Show>
                          </td>
                          <td>
                            <input
                              type="text"
                              inputMode="decimal"
                              name="lowLevel"
                              data-field="lowLevel"
                              value={row().lowLevel}
                              onInput={(e) => {
                                const val = e.currentTarget.value;
                                setRows((prev) =>
                                  prev.map((r, i) =>
                                    i === index ? { ...r, lowLevel: val } : r,
                                  ),
                                );
                              }}
                            />
                          </td>
                          <td>
                            <button
                              type="button"
                              data-action="remove-level"
                              onClick={() => removeRow(index)}
                            >
                              {t('levelRemove')}
                            </button>
                          </td>
                        </tr>
                      )}
                    </Index>
                  </tbody>
                </table>
              </Show>

              <div class="cash-limits-actions drawer-low-levels-actions">
                <button
                  type="button"
                  data-action="add-level"
                  onClick={addRow}
                >
                  {t('levelAdd')}
                </button>
                <button
                  type="submit"
                  class="primary"
                  data-action="save-levels"
                  disabled={busy()}
                >
                  {t('levelsSave')}
                </button>
              </div>
            </form>

            <Show when={refusal()}>
              <p role="alert" class="error-message" data-form-error data-levels-error>
                {refusal()}
              </p>
            </Show>
          </CardBody>
        </Card>
      </div>
    </Show>
  );
}
