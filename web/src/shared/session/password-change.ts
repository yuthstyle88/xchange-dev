let redirected = false;

export function needsPasswordChange(status: number, code: string | undefined): boolean {
  return status === 403 && code === 'passwordChangeRequired';
}

export function passwordChangeUrl(here: string): string {
  return '/login?next=' + encodeURIComponent(here);
}

export function goChangePassword(): void {
  if (redirected || typeof window === 'undefined') return;
  const { pathname, search, hash } = window.location;
  if (pathname === '/login') return;
  redirected = true;
  window.location.assign(passwordChangeUrl(pathname + search + hash));
}
