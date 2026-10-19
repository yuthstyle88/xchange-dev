// B03 customer step loyalty block (FE-desk skeleton, #564; design membership-points.md §6.3 CU-P1, §4.6 `customer_tier`).
// The desk design (#536) leaves the phone step's place open; the FE-desk sub-controller placed it in the summary aside,
// shown on every step once `store.customerId()` is set (steps/customer/step.tsx sets it before `go(2)`). It composes two
// parts: phone verification and the tier badge, which reads again after a phone is verified.
import { createSignal, Show, type JSX } from 'solid-js';
import type { DeskStore } from '../desk-store';
import { PhoneVerify } from './phone/PhoneVerify';
import { TierBadge } from './tier/TierBadge';

export function CustomerLoyalty(props: { store: DeskStore }): JSX.Element {
  const [verifiedCount, setVerifiedCount] = createSignal(0);
  return (
    <Show when={props.store.customerId()}>
      {(customerId) => (
        <div>
          <TierBadge api={props.store.api} branch={props.store.branch()} customerId={customerId()} refreshKey={verifiedCount()} />
          <PhoneVerify
            api={props.store.api}
            branch={props.store.branch()}
            customerId={customerId()}
            onVerified={() => setVerifiedCount((n) => n + 1)}
          />
        </div>
      )}
    </Show>
  );
}
