export function can(
  me: { permissions: readonly string[] } | undefined,
  permission: string
): boolean {
  return me?.permissions.includes(permission) ?? false;
}
