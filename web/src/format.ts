// Display formatting. Values arrive as exact decimal strings; these only shorten them for the screen.

/** Compact volume: `0.086`, `12.5`, `1.2k`, `3.4M`. */
export function volume(value: string): string {
  const n = Number(value);
  const abs = Math.abs(n);
  if (abs >= 1e6) return `${trim((n / 1e6).toFixed(1))}M`;
  if (abs >= 1e3) return `${trim((n / 1e3).toFixed(1))}k`;
  if (abs >= 100) return trim(n.toFixed(0));
  if (abs >= 1) return trim(n.toFixed(2));
  return trim(n.toFixed(3));
}

/** Signed volume, e.g. `+1.2k` / `-0.4`. */
export function signedVolume(value: string): string {
  const text = volume(value);
  return Number(value) > 0 ? `+${text}` : text;
}

/** Number of decimals in a decimal string (`0.10` → 2). */
export function decimals(value: string): number {
  const dot = value.indexOf(".");
  return dot < 0 ? 0 : value.length - dot - 1;
}

/** `HH:MM` (UTC) of a microsecond timestamp. */
export function clockUtc(us: number): string {
  return new Date(us / 1000).toISOString().slice(11, 16);
}

/** `YYYY-MM-DD HH:MM UTC`. */
export function dateTimeUtc(us: number): string {
  const iso = new Date(us / 1000).toISOString();
  return `${iso.slice(0, 10)} ${iso.slice(11, 16)} UTC`;
}

function trim(text: string): string {
  return text.includes(".") ? text.replace(/\.?0+$/, "") : text;
}
