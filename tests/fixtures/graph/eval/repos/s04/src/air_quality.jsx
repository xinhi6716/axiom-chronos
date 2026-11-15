export function dailyReading(readings, offset) {
  return readings.reduce((sum, value) => sum + value + offset, 0) / readings.length;
}
