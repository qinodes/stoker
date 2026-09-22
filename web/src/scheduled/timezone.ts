export interface LocalDateTime {
  date: string;
  time: string;
}

export type LocalDateTimeConversion =
  | { ok: true; value: string }
  | { ok: false; reason: "invalid" | "nonexistent" };

interface ZonedParts {
  year: number;
  month: number;
  day: number;
  hour: number;
  minute: number;
  second: number;
}

const HOUR_MS = 60 * 60 * 1000;

export function instantToLocalDateTime(instant: string, timezone: string): LocalDateTime | null {
  const timestamp = Date.parse(instant);
  if (!Number.isFinite(timestamp)) return null;
  try {
    const parts = zonedParts(timestamp, timezone);
    return {
      date: `${parts.year}-${pad(parts.month)}-${pad(parts.day)}`,
      time: `${pad(parts.hour)}:${pad(parts.minute)}`,
    };
  } catch {
    return null;
  }
}

export function localDateTimeToRfc3339(date: string, time: string, timezone: string): LocalDateTimeConversion {
  const dateMatch = /^(\d{4})-(\d{2})-(\d{2})$/.exec(date);
  const timeMatch = /^(\d{2}):(\d{2})$/.exec(time);
  if (!dateMatch || !timeMatch) return { ok: false, reason: "invalid" };
  const target: ZonedParts = {
    year: Number(dateMatch[1]), month: Number(dateMatch[2]), day: Number(dateMatch[3]),
    hour: Number(timeMatch[1]), minute: Number(timeMatch[2]), second: 0,
  };
  const nominalUtc = Date.UTC(target.year, target.month - 1, target.day, target.hour, target.minute);
  const normalized = new Date(nominalUtc);
  if (normalized.getUTCFullYear() !== target.year || normalized.getUTCMonth() + 1 !== target.month || normalized.getUTCDate() !== target.day || normalized.getUTCHours() !== target.hour || normalized.getUTCMinutes() !== target.minute) {
    return { ok: false, reason: "invalid" };
  }

  try {
    const offsets = new Set<number>();
    for (let hour = -36; hour <= 36; hour += 1) {
      const sample = nominalUtc + hour * HOUR_MS;
      const viewed = zonedParts(sample, timezone);
      offsets.add(Date.UTC(viewed.year, viewed.month - 1, viewed.day, viewed.hour, viewed.minute, viewed.second) - sample);
    }
    const candidates = [...offsets]
      .map((offset) => nominalUtc - offset)
      .filter((candidate) => sameParts(zonedParts(candidate, timezone), target))
      .sort((left, right) => left - right);
    if (!candidates.length) return { ok: false, reason: "nonexistent" };
    return { ok: true, value: new Date(candidates[0]).toISOString().replace(".000Z", "Z") };
  } catch {
    return { ok: false, reason: "invalid" };
  }
}

function zonedParts(timestamp: number, timezone: string): ZonedParts {
  const formatter = new Intl.DateTimeFormat("en-US-u-ca-iso8601", {
    timeZone: timezone,
    year: "numeric", month: "2-digit", day: "2-digit",
    hour: "2-digit", minute: "2-digit", second: "2-digit", hourCycle: "h23",
  });
  const values = Object.fromEntries(formatter.formatToParts(new Date(timestamp)).map((part) => [part.type, part.value]));
  return {
    year: Number(values.year), month: Number(values.month), day: Number(values.day),
    hour: Number(values.hour), minute: Number(values.minute), second: Number(values.second),
  };
}

function sameParts(left: ZonedParts, right: ZonedParts): boolean {
  return left.year === right.year && left.month === right.month && left.day === right.day
    && left.hour === right.hour && left.minute === right.minute && left.second === right.second;
}

function pad(value: number): string {
  return String(value).padStart(2, "0");
}
