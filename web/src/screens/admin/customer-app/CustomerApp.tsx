// #6286: the owner sets which OTP channels a customer may choose, in order; the first is preselected.
// Reads `GET /api/auth/otp/channels`, saves through `PUT /api/app-settings` (`otpChannels`, `null` = default).
import { createEffect, createSignal, For, Show, type JSX } from 'solid-js';
import { ApiClient } from '../../../shared/api/client';
import { keyedWrite } from '../../../shared/api/keyed-write';
import type {
  AppSettingsSaveInput,
  AppSettingsSaveOutput,
} from '../../../shared/api/generated/app_settings_save';
import { screenT } from '../../../shared/i18n';
import { wordedError } from '../../../shared/i18n/worded-error';
import type { AdminMenuProps } from '../menus';
import * as table from './i18n';
import '../../../shared/ui/card.css';
import '../../../shared/ui/list-states.css';

const t = screenT('admin.customer-app', table);

type Channel = 'email' | 'sms';
const ALL: Channel[] = ['email', 'sms'];

export function CustomerApp(props: AdminMenuProps): JSX.Element {
  const api = new ApiClient();
  const saveWrite = keyedWrite<AppSettingsSaveInput, AppSettingsSaveOutput>(
    api,
    'PUT',
    '/api/app-settings',
  );

  // Every channel in display order; `ticked` = the ones customers may choose.
  const [order, setOrder] = createSignal<Channel[]>(ALL);
  const [ticked, setTicked] = createSignal<Channel[]>([]);
  const [source, setSource] = createSignal<'set' | 'default' | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  const [notice, setNotice] = createSignal<string | null>(null);
  const [saving, setSaving] = createSignal(false);

  const load = async () => {
    try {
      const res = await api.publicGet<{ channels: string[]; source: 'set' | 'default' }>(
        '/api/auth/otp/channels',
      );
      const chosen = res.channels.filter((c): c is Channel => ALL.includes(c as Channel));
      setOrder([...chosen, ...ALL.filter((c) => !chosen.includes(c))]);
      setTicked(chosen);
      setSource(res.source);
    } catch (err: unknown) {
      setError(wordedError(err));
    }
  };

  createEffect(() => {
    if (props.shown()) void load();
  });

  const toggle = (c: Channel, on: boolean) =>
    setTicked((prev) => (on ? [...prev, c] : prev.filter((x) => x !== c)));

  const moveFirst = (c: Channel) => setOrder((prev) => [c, ...prev.filter((x) => x !== c)]);

  const save = async (otpChannels: string[] | null) => {
    setError(null);
    setNotice(null);
    setSaving(true);
    try {
      await saveWrite.send({ otpChannels });
      setNotice(t('saved'));
      await load();
    } catch (err: unknown) {
      setError(wordedError(err));
    } finally {
      setSaving(false);
    }
  };

  const chosenInOrder = () => order().filter((c) => ticked().includes(c));

  return (
    <div class="card">
      <div class="card-heading">
        <h3>{t('otpHeading')}</h3>
        <Show when={source()}>
          {(s) => (
            <span class="form-hint" data-otp-source={s()}>
              {s() === 'default' ? t('sourceDefault') : t('sourceSet')}
            </span>
          )}
        </Show>
      </div>
      <div class="card-body">
        <p class="form-hint">{t('otpHint')}</p>
        <For each={order()}>
          {(c, i) => (
            <div>
              <label class="check-field">
                <input
                  type="checkbox"
                  data-otp-channel={c}
                  checked={ticked().includes(c)}
                  onChange={(e) => toggle(c, e.currentTarget.checked)}
                />
                <span class="check-text">{t(c)}</span>
              </label>
              <Show when={i() > 0}>
                <button type="button" onClick={() => moveFirst(c)}>
                  {t('moveFirst')}
                </button>
              </Show>
            </div>
          )}
        </For>
        <Show when={ticked().length === 0}>
          <p class="form-hint">{t('pickOne')}</p>
        </Show>
        <Show when={error()}>
          <p class="xc-list-error" role="alert">
            {error()}
          </p>
        </Show>
        <Show when={notice()}>
          <p class="form-hint" role="status">
            {notice()}
          </p>
        </Show>
        <div>
          <button
            type="button"
            disabled={saving() || ticked().length === 0}
            onClick={() => void save(chosenInOrder())}
          >
            {t('save')}
          </button>
          <Show when={source() === 'set'}>
            <button type="button" disabled={saving()} onClick={() => void save(null)}>
              {t('reset')}
            </button>
          </Show>
        </div>
      </div>
    </div>
  );
}
