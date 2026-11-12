export function isExpired(expires, now) {
  return now >= expires;
}
