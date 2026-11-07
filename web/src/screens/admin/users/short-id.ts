const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
export function shortId(id: string): string {
  return UUID.test(id) ? id.slice(0, 8) : id;
}
