// A customer page whose read answered 401 (signed out) shows what the visitor needs and one wine "Sign in" button
// back to the existing sign-in page, with `next=` so a sign-in returns to this page (SignIn.tsx nextTarget). Before
// this, such a page showed the server's "กรุณาเข้าสู่ระบบ" and only a Retry button, which could never succeed.
import type { JSX } from 'solid-js';
import { screenT } from '../../../shared/i18n';
import * as table from './i18n';

const tr = screenT('customer._shell', table);

/** True when a failed read is the server's "signed out" answer. */
export function isSignedOut(err: unknown): boolean {
  return (err as { status?: number } | null | undefined)?.status === 401;
}

/** The sign-in page's address with `next` set to `next` (default: this page's path and query). */
export function signInHref(next?: string): string {
  const back = next ?? `${window.location.pathname}${window.location.search}`;
  return `/app/sign-in?next=${encodeURIComponent(back)}`;
}

/**
 * Wraps a client so a `401` from any of its calls runs `onSignedOut`. The calls themselves, and what they answer or
 * throw, are unchanged; it lets a screen whose read lives in a shared part (createNotifications) see the status.
 */
export function watchSignedOut<T extends object>(client: T, onSignedOut: () => void): T {
  return new Proxy(client, {
    get(target, prop) {
      const value = Reflect.get(target, prop, target) as unknown;
      if (typeof value !== 'function') return value;
      return (...args: unknown[]) => {
        const out = (value as (...a: unknown[]) => unknown).apply(target, args);
        if (out instanceof Promise) {
          return out.catch((err: unknown) => {
            if (isSignedOut(err)) onSignedOut();
            throw err;
          });
        }
        return out;
      };
    },
  });
}

export function SignInNeeded(props: { message?: string; next?: string }): JSX.Element {
  return (
    <div class="xc-c-signin-needed" data-signin-needed>
      <p>{props.message ?? tr('signInNeeded')}</p>
      <a class="xc-c-primary" href={signInHref(props.next)}>
        {tr('signIn')}
      </a>
    </div>
  );
}
