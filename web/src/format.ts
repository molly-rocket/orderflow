export function volume(value: string): string {
  const n = Number(value);
  const abs = Math.abs(n);
  if (abs >= 1e6) return `${trim((n / 1e6).toFixed(1))}M`;
  if (abs >= 1e3) return `${trim((n / 1e3).toFixed(1))}k`;
  if (abs >= 100) return trim(n.toFixed(0));
  if (abs >= 1) return trim(n.toFixed(2));
  return trim(n.toFixed(3));
}

export function signedVolume(value: string): string {
  const text = volume(value);
  return Number(value) > 0 ? `+${text}` : text;
}

export function usd(value: string | number): string {
  const n = Number(value);
  const abs = Math.abs(n);
  if (abs >= 1e9) return `$${trim((n / 1e9).toFixed(2))}B`;
  if (abs >= 1e6) return `$${trim((n / 1e6).toFixed(2))}M`;
  if (abs >= 1e3) return `$${trim((n / 1e3).toFixed(1))}k`;
  return `$${trim(n.toFixed(2))}`;
}

export function price(value: string | number, decimals: number): string {
  return Number(value).toLocaleString("en-US", {
    minimumFractionDigits: decimals,
    maximumFractionDigits: decimals,
  });
}

export function decimals(value: string): number {
  const dot = value.indexOf(".");
  return dot < 0 ? 0 : value.length - dot - 1;
}

export function clockUtc(us: number): string {
  return new Date(us / 1000).toISOString().slice(11, 16);
}

export function preciseClockUtc(us: number): string {
  return `${new Date(us / 1000).toISOString().slice(11, 23)} UTC`;
}

export function dayLabelUtc(us: number): string {
  return new Date(us / 1000).toLocaleDateString("en-US", { month: "short", day: "numeric", timeZone: "UTC" });
}

export function dateTimeUtc(us: number): string {
  const iso = new Date(us / 1000).toISOString();
  return `${iso.slice(0, 10)} ${iso.slice(11, 16)} UTC`;
}

function trim(text: string): string {
  return text.includes(".") ? text.replace(/\.?0+$/, "") : text;
}
