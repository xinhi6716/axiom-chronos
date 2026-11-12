export function shippingCost(weightKg: number): number {
  return weightKg > 10 ? 20 : 8;
}
