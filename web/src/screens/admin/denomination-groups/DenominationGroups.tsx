import { createEffect, createSignal, For, Show, type JSX } from 'solid-js';
import { ApiClient } from '../../../shared/api/client';
import type {
  AdminDenominationGroupsData,
  GroupDto,
} from '../../../shared/api/generated/admin_denomination_groups';
import { keyedWrite } from '../../../shared/api/keyed-write';
import { screenT } from '../../../shared/i18n';
import { wordedError } from '../../../shared/i18n/worded-error';
import type { AdminMenuProps } from '../menus';
import { toast } from '../parts/toast';
import * as table from './i18n';
import './denomination-groups.css';

const t = screenT('admin.denomination-groups', table);

interface GroupRowProps {
  group: GroupDto;
  currency: string;
  provider: () => string;
  branch: () => string;
  client: ApiClient;
  onRefresh: () => Promise<void>;
}

function GroupRow(props: GroupRowProps): JSX.Element {
  const mapWrite = keyedWrite(props.client, 'POST', '/api/denomination-groups/map');
  const removeWrite = keyedWrite(props.client, 'POST', '/api/denomination-groups/remove');
  const [boardKey, setBoardKey] = createSignal(props.group.instrumentKey ?? '');
  const [error, setError] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal(false);

  createEffect(() => {
    setBoardKey(props.group.instrumentKey ?? '');
  });

  const handleSave = async () => {
    const key = boardKey();
    if (key.trim() === '') {
      const msg = t('keyRequired');
      setError(msg);
      toast(msg);
      return;
    }

    const provider = props.provider();
    if (!provider || provider.trim() === '') {
      const msg = t('providerRequired');
      setError(msg);
      toast(msg);
      return;
    }

    setBusy(true);
    try {
      await mapWrite.send({
        currency: props.currency,
        group: props.group.group,
        provider,
        instrumentKey: key,
        branch: props.branch(),
      });
      setError(null);
      await props.onRefresh();
    } catch (err: unknown) {
      const message = wordedError(err);
      setError(message);
      toast(message);
    } finally {
      setBusy(false);
    }
  };

  const handleRemove = async () => {
    setBusy(true);
    try {
      await removeWrite.send({
        currency: props.currency,
        group: props.group.group,
        branch: props.branch(),
      });
      setError(null);
      await props.onRefresh();
    } catch (err: unknown) {
      const message = wordedError(err);
      setError(message);
      toast(message);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div data-group={props.group.group} class="group-row">
      <span class="group-code">{props.group.group}</span>
      <span class="board-key-display">{props.group.instrumentKey ?? ''}</span>
      <label>
        {t('boardKey')}
        <input
          type="text"
          value={boardKey()}
          onInput={(e) => {
            setBoardKey(e.currentTarget.value);
            setError(null);
          }}
          onChange={(e) => setBoardKey(e.currentTarget.value)}
          disabled={busy()}
        />
      </label>
      <button type="button" onClick={handleSave} disabled={busy()}>
        {t('save')}
      </button>
      <Show when={props.group.removable}>
        <button type="button" onClick={handleRemove} disabled={busy()}>
          {t('remove')}
        </button>
      </Show>
      <Show when={error()}>
        <div class="row-error">{error()}</div>
      </Show>
    </div>
  );
}

export function DenominationGroups(props: AdminMenuProps): JSX.Element {
  const client = new ApiClient();
  const addGroupWrite = keyedWrite(client, 'POST', '/api/denomination-groups');

  const [data, setData] = createSignal<AdminDenominationGroupsData | null>(null);
  const [selectedCurrency, setSelectedCurrency] = createSignal('');
  const [newGroup, setNewGroup] = createSignal('');
  const [addError, setAddError] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal(false);
  let loadSeq = 0;

  const loadView = async () => {
    const seq = ++loadSeq;
    try {
      const res = await client.view<AdminDenominationGroupsData>(
        'admin_denomination_groups',
        {
          branch: props.branch(),
        },
      );
      if (seq !== loadSeq) return;
      setData(res.data ?? null);
    } catch (err: unknown) {
      if (seq !== loadSeq) return;
      setData(null);
      const message = wordedError(err);
      toast(message);
    }
  };

  createEffect(() => {
    props.branch();
    setData(null);
    loadView();
  });

  const currentCurrency = () =>
    data()?.currencies.find((c) => c.currency === selectedCurrency());

  const handleAddGroup = async () => {
    const code = newGroup();
    if (!/^[A-Z0-9_]{1,32}$/.test(code)) {
      const msg = t('groupInvalid');
      setAddError(msg);
      toast(msg);
      return;
    }
    const curr = selectedCurrency();
    if (!curr) return;

    setBusy(true);
    try {
      await addGroupWrite.send({
        currency: curr,
        group: code,
        branch: props.branch(),
      });
      setNewGroup('');
      setAddError(null);
      await loadView();
    } catch (err: unknown) {
      const message = wordedError(err);
      setAddError(message);
      toast(message);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div class="denomination-groups-screen">
      <label class="denomination-groups-currency-picker">
        {t('currency')}
        <select
          value={selectedCurrency()}
          onInput={(e) => setSelectedCurrency(e.currentTarget.value)}
          onChange={(e) => setSelectedCurrency(e.currentTarget.value)}
        >
          <option value="" disabled selected={selectedCurrency() === ''}>
            {t('currency')}
          </option>
          <For each={data()?.currencies ?? []}>
            {(c) => <option value={c.currency}>{c.currency}</option>}
          </For>
        </select>
      </label>

      <Show when={selectedCurrency() !== ''}>
        <div class="denomination-groups-list">
          <For each={currentCurrency()?.groups ?? []}>
            {(g) => (
              <GroupRow
                group={g}
                currency={selectedCurrency()}
                provider={() => data()?.provider ?? ''}
                branch={props.branch}
                client={client}
                onRefresh={loadView}
              />
            )}
          </For>
        </div>

        <div class="add-group-form">
          <label>
            {t('newGroup')}
            <input
              type="text"
              value={newGroup()}
              onInput={(e) => {
                setNewGroup(e.currentTarget.value);
                setAddError(null);
              }}
              onChange={(e) => setNewGroup(e.currentTarget.value)}
              disabled={busy()}
            />
          </label>
          <button type="button" onClick={handleAddGroup} disabled={busy()}>
            {t('add')}
          </button>
          <Show when={addError()}>
            <div class="error-msg">{addError()}</div>
          </Show>
        </div>
      </Show>
    </div>
  );
}
