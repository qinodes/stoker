export function policyRouteKey(key: string): string {
  return key.replaceAll("_", "-");
}

export function policyInputIsValid(value: string, allowZero = false): boolean {
  return /^\d+$/.test(value) && Number.isSafeInteger(Number(value)) && (allowZero ? Number(value) >= 0 : Number(value) > 0);
}

export function formatPolicyValue(value: number | null, unit: string, locale?: string, disabled = "Disabled"): string {
  if (value === null) return disabled;
  return `${value.toLocaleString(locale)} ${unit}`;
}
