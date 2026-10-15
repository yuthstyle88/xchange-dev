// BK-9 (#1410): Admin Booking Slots screen (design `docs/design/blueprint/booking-slots.md` §4.4, §7, §8, §9 BK-9).
import { createEffect, createSignal, For, Show, type JSX } from 'solid-js';
import { ApiClient, ApiError } from '../../../shared/api/client';
import { screenT } from '../../../shared/i18n';
import type { AdminMenuProps } from '../menus';
import type {
  AdminBookingSlotsData,
  AdminClosureDto,
  AdminSlotDto,
} from '../../../shared/api/generated/admin_booking_slots';
import type {
  BookingPolicySetInput,
  ClosureAddInput,
  SlotCapacitySetInput,
  SlotRuleSetInput,
} from '../../../shared/api/generated/bookings';
import * as table from './i18n';
import './booking-slots.css';

const pendingKeys = new Map<string, { bodyString: string; key: string }>();

function getIdempotencyKey(actionId: string, bodyString: string): string {
  const existing = pendingKeys.get(actionId);
  if (existing && existing.bodyString === bodyString) {
    return existing.key;
  }
  const newKey = crypto.randomUUID();
  pendingKeys.set(actionId, { bodyString, key: newKey });
  return newKey;
}

function clearIdempotencyKey(actionId: string): void {
  pendingKeys.delete(actionId);
}

export function BookingSlots(props: AdminMenuProps): JSX.Element {
  const t = screenT('admin.booking-slots', table);
  const api = new ApiClient();

  const [loading, setLoading] = createSignal(false);
  const [errorMessage, setErrorMessage] = createSignal<string | null>(null);

  // Slot Rule
  const [slotMinutes, setSlotMinutes] = createSignal('');
  const [capacity, setCapacity] = createSignal('');
  const [horizonDays, setHorizonDays] = createSignal('');

  // Closures
  const [closures, setClosures] = createSignal<AdminClosureDto[]>([]);
  const [newClosedOn, setNewClosedOn] = createSignal('');
  const [newReason, setNewReason] = createSignal('');

  // Slots
  const [slots, setSlots] = createSignal<AdminSlotDto[]>([]);
  const [slotCapacities, setSlotCapacities] = createSignal<Record<string, string>>({});

  // Policy
  const [cutoff, setCutoff] = createSignal('');
  const [grace, setGrace] = createSignal('');
  const [expiry, setExpiry] = createSignal('');

  async function loadView(branchId: string) {
    setLoading(true);
    setErrorMessage(null);
    try {
      const res = await api.view<AdminBookingSlotsData>('admin_booking_slots', { branch: branchId });
      const data = res.data;
      if (data) {
        if (data.rule) {
          setSlotMinutes(String(data.rule.slotMinutes));
          setCapacity(String(data.rule.capacity));
          setHorizonDays(String(data.rule.horizonDays));
        } else {
          setSlotMinutes('');
          setCapacity('');
          setHorizonDays('');
        }
        setClosures(data.closures ?? []);
        setSlots(data.slots ?? []);
        if (data.policy) {
          setCutoff(String(data.policy.cancelCutoffMinutes));
          setGrace(String(data.policy.noShowGraceMinutes));
          setExpiry(String(data.policy.confirmExpiryMinutes));
        }
        if (data.slots) {
          const caps: Record<string, string> = {};
          for (const s of data.slots) {
            caps[s.id] = String(s.capacity);
          }
          setSlotCapacities(caps);
        }
      }
    } catch (err: unknown) {
      handleApiError(err);
    } finally {
      setLoading(false);
    }
  }

  createEffect(() => {
    const branch = props.branch();
    if (branch) {
      void loadView(branch);
    }
  });

  function handleApiError(err: unknown) {
    if (err instanceof ApiError) {
      const code = err.code;
      const errorKey = `error.${code}`;
      if (errorKey in table.th) {
        setErrorMessage(t(errorKey as any));
        return;
      }
      const errKey = `err.${code}`;
      if (errKey in table.th) {
        setErrorMessage(t(errKey as any));
        return;
      }
    }
    setErrorMessage(t('error.validationFailed'));
  }

  async function handleSaveRule() {
    setErrorMessage(null);
    const sMin = Number(slotMinutes());
    const cap = Number(capacity());
    const hDays = Number(horizonDays());

    // Boundary checks: slot_minutes 5-240, capacity > 0, horizon_days 1-90
    if (
      !Number.isInteger(sMin) ||
      sMin < 5 ||
      sMin > 240 ||
      !Number.isInteger(cap) ||
      cap <= 0 ||
      !Number.isInteger(hDays) ||
      hDays < 1 ||
      hDays > 90
    ) {
      setErrorMessage(t('ruleValidationFailed'));
      return;
    }

    const payload = {
      slotMinutes: sMin,
      capacity: cap,
      horizonDays: hDays,
      branch: props.branch(),
    } satisfies Omit<SlotRuleSetInput, 'id'> & { branch: string };
    const bodyString = JSON.stringify(payload);
    const actionId = `slotRuleSet:${props.branch()}`;
    const key = getIdempotencyKey(actionId, bodyString);

    try {
      await api.command(
        'PUT',
        `/api/branches/${encodeURIComponent(props.branch())}/booking-slot-rule`,
        payload,
        key,
      );
      clearIdempotencyKey(actionId);
      await loadView(props.branch());
    } catch (err: unknown) {
      handleApiError(err);
    }
  }

  async function handleAddClosure() {
    setErrorMessage(null);
    const closedOn = newClosedOn().trim();
    const reason = newReason().trim();

    if (!closedOn || !reason) {
      setErrorMessage(t('closureValidationFailed'));
      return;
    }

    const payload = {
      closedOn,
      reason,
      branch: props.branch(),
    } satisfies Omit<ClosureAddInput, 'id'> & { branch: string };
    const bodyString = JSON.stringify(payload);
    const actionId = `closureAdd:${props.branch()}:${closedOn}`;
    const key = getIdempotencyKey(actionId, bodyString);

    try {
      await api.command(
        'POST',
        `/api/branches/${encodeURIComponent(props.branch())}/closures`,
        payload,
        key,
      );
      clearIdempotencyKey(actionId);
      setNewClosedOn('');
      setNewReason('');
      await loadView(props.branch());
    } catch (err: unknown) {
      handleApiError(err);
    }
  }

  async function handleRemoveClosure(closedOn: string) {
    setErrorMessage(null);
    const payload = { branch: props.branch() };
    const bodyString = JSON.stringify(payload);
    const actionId = `closureRemove:${props.branch()}:${closedOn}`;
    const key = getIdempotencyKey(actionId, bodyString);

    try {
      await api.command(
        'POST',
        `/api/branches/${encodeURIComponent(props.branch())}/closures/${encodeURIComponent(closedOn)}/remove`,
        payload,
        key,
      );
      clearIdempotencyKey(actionId);
      await loadView(props.branch());
    } catch (err: unknown) {
      handleApiError(err);
    }
  }

  async function handleUpdateSlotCapacity(slotId: string) {
    setErrorMessage(null);
    const capVal = slotCapacities()[slotId];
    const capNum = Number(capVal);

    if (!Number.isInteger(capNum) || capNum <= 0) {
      setErrorMessage(t('slotCapacityValidationFailed'));
      return;
    }

    const payload = {
      capacity: capNum,
      branch: props.branch(),
    } satisfies Omit<SlotCapacitySetInput, 'id'> & { branch: string };
    const bodyString = JSON.stringify(payload);
    const actionId = `slotCapacitySet:${slotId}`;
    const key = getIdempotencyKey(actionId, bodyString);

    try {
      await api.command(
        'PUT',
        `/api/booking-slots/${encodeURIComponent(slotId)}/capacity`,
        payload,
        key,
      );
      clearIdempotencyKey(actionId);
      await loadView(props.branch());
    } catch (err: unknown) {
      handleApiError(err);
    }
  }

  async function handleCloseSlot(slotId: string) {
    setErrorMessage(null);
    const payload = { branch: props.branch() };
    const bodyString = JSON.stringify(payload);
    const actionId = `slotClose:${slotId}`;
    const key = getIdempotencyKey(actionId, bodyString);

    try {
      await api.command(
        'POST',
        `/api/booking-slots/${encodeURIComponent(slotId)}/close`,
        payload,
        key,
      );
      clearIdempotencyKey(actionId);
      await loadView(props.branch());
    } catch (err: unknown) {
      handleApiError(err);
    }
  }

  async function handleOpenSlot(slotId: string) {
    setErrorMessage(null);
    const payload = { branch: props.branch() };
    const bodyString = JSON.stringify(payload);
    const actionId = `slotOpen:${slotId}`;
    const key = getIdempotencyKey(actionId, bodyString);

    try {
      await api.command(
        'POST',
        `/api/booking-slots/${encodeURIComponent(slotId)}/open`,
        payload,
        key,
      );
      clearIdempotencyKey(actionId);
      await loadView(props.branch());
    } catch (err: unknown) {
      handleApiError(err);
    }
  }

  async function handleSavePolicy() {
    setErrorMessage(null);
    const cut = Number(cutoff());
    const grc = Number(grace());
    const exp = Number(expiry());

    // Boundary checks: cutoff >= 0, grace >= 0, expiry > 0
    if (
      !Number.isInteger(cut) ||
      cut < 0 ||
      !Number.isInteger(grc) ||
      grc < 0 ||
      !Number.isInteger(exp) ||
      exp <= 0
    ) {
      setErrorMessage(t('policyValidationFailed'));
      return;
    }

    const payload = {
      cancelCutoffMinutes: cut,
      noShowGraceMinutes: grc,
      confirmExpiryMinutes: exp,
      branch: props.branch(),
    } satisfies BookingPolicySetInput & { branch: string };
    const bodyString = JSON.stringify(payload);
    const actionId = 'bookingPolicySet';
    const key = getIdempotencyKey(actionId, bodyString);

    try {
      await api.command(
        'PUT',
        '/api/booking-policy',
        payload,
        key,
      );
      clearIdempotencyKey(actionId);
      await loadView(props.branch());
    } catch (err: unknown) {
      handleApiError(err);
    }
  }

  return (
    <div class="booking-slots-screen">
      <Show when={loading()}>
        <div class="loading">{t('loading')}</div>
      </Show>

      <Show when={errorMessage()}>
        <div role="alert" class="error field-error">
          {errorMessage()}
        </div>
      </Show>

      {/* Section 1: Slot Rule */}
      <section class="booking-slots-section">
        <h3>{t('ruleSection')}</h3>
        <div class="form-group">
          <label for="slotMinutes">{t('slotMinutes')}</label>
          <input
            id="slotMinutes"
            name="slotMinutes"
            data-field="slotMinutes"
            type="number"
            value={slotMinutes()}
            onInput={(e) => setSlotMinutes(e.currentTarget.value)}
          />
        </div>
        <div class="form-group">
          <label for="capacity">{t('capacity')}</label>
          <input
            id="capacity"
            name="capacity"
            data-field="capacity"
            type="number"
            value={capacity()}
            onInput={(e) => setCapacity(e.currentTarget.value)}
          />
        </div>
        <div class="form-group">
          <label for="horizonDays">{t('horizonDays')}</label>
          <input
            id="horizonDays"
            name="horizonDays"
            data-field="horizonDays"
            type="number"
            value={horizonDays()}
            onInput={(e) => setHorizonDays(e.currentTarget.value)}
          />
        </div>
        <button type="button" onClick={handleSaveRule}>
          {t('saveRule')}
        </button>
      </section>

      {/* Section 2: Branch Closures */}
      <section class="booking-slots-section">
        <h3>{t('closuresSection')}</h3>
        <div class="closures-form">
          <div class="form-group">
            <label for="closedOn">{t('closedOn')}</label>
            <input
              id="closedOn"
              name="closedOn"
              data-field="closedOn"
              type="date"
              value={newClosedOn()}
              onInput={(e) => setNewClosedOn(e.currentTarget.value)}
            />
          </div>
          <div class="form-group">
            <label for="reason">{t('reason')}</label>
            <input
              id="reason"
              name="reason"
              data-field="reason"
              type="text"
              value={newReason()}
              onInput={(e) => setNewReason(e.currentTarget.value)}
            />
          </div>
          <button type="button" onClick={handleAddClosure}>
            {t('addClosure')}
          </button>
        </div>

        <div class="closures-list">
          <For each={closures()} fallback={<div>{t('noClosures')}</div>}>
            {(closure) => (
              <div class="closure-item">
                <span class="closure-date">{closure.closedOn}</span>
                <span class="closure-reason">{closure.reason}</span>
                <button type="button" onClick={() => void handleRemoveClosure(closure.closedOn)}>
                  {t('removeClosure')}
                </button>
              </div>
            )}
          </For>
        </div>
      </section>

      {/* Section 3: Booking Slots */}
      <section class="booking-slots-section">
        <h3>{t('slotsSection')}</h3>
        <div class="slots-list">
          <For each={slots()} fallback={<div>{t('noSlots')}</div>}>
            {(slot) => (
              <div class="slot-item" data-slot-id={slot.id}>
                <span class="slot-date">{slot.serviceDate}</span>
                <span class="slot-time">
                  {slot.startAt} - {slot.endAt}
                </span>
                <span class="slot-status">{slot.status}</span>
                <span class="slot-reserved">
                  {t('reserved')}: {slot.reserved}
                </span>
                <div class="slot-capacity-control">
                  <input
                    name="slotCapacity"
                    data-slot-id={slot.id}
                    type="number"
                    value={slotCapacities()[slot.id] ?? slot.capacity}
                    onInput={(e) => {
                      setSlotCapacities((prev) => ({
                        ...prev,
                        [slot.id]: e.currentTarget.value,
                      }));
                    }}
                  />
                  <button type="button" onClick={() => void handleUpdateSlotCapacity(slot.id)}>
                    {t('updateCapacity')}
                  </button>
                </div>
                <button type="button" onClick={() => void handleCloseSlot(slot.id)}>
                  {t('close')}
                </button>
                <button type="button" onClick={() => void handleOpenSlot(slot.id)}>
                  {t('open')}
                </button>
              </div>
            )}
          </For>
        </div>
      </section>

      {/* Section 4: Booking Policy */}
      <section class="booking-slots-section">
        <h3>{t('policySection')}</h3>
        <div class="form-group">
          <label for="cancelCutoffMinutes">{t('cancelCutoffMinutes')}</label>
          <input
            id="cancelCutoffMinutes"
            name="cancelCutoffMinutes"
            data-field="cancelCutoffMinutes"
            type="number"
            value={cutoff()}
            onInput={(e) => setCutoff(e.currentTarget.value)}
          />
        </div>
        <div class="form-group">
          <label for="noShowGraceMinutes">{t('noShowGraceMinutes')}</label>
          <input
            id="noShowGraceMinutes"
            name="noShowGraceMinutes"
            data-field="noShowGraceMinutes"
            type="number"
            value={grace()}
            onInput={(e) => setGrace(e.currentTarget.value)}
          />
        </div>
        <div class="form-group">
          <label for="confirmExpiryMinutes">{t('confirmExpiryMinutes')}</label>
          <input
            id="confirmExpiryMinutes"
            name="confirmExpiryMinutes"
            data-field="confirmExpiryMinutes"
            type="number"
            value={expiry()}
            onInput={(e) => setExpiry(e.currentTarget.value)}
          />
        </div>
        <button type="button" onClick={handleSavePolicy}>
          {t('savePolicy')}
        </button>
      </section>
    </div>
  );
}
