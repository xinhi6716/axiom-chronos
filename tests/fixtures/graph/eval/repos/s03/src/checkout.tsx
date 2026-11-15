export function applyCoupon(total: number, expires: number, now: number): number {
  return now > expires ? total : total * 0.9;
}
