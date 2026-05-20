/**
 * Cron expression validation, description, and next-fire-time helpers.
 *
 * Mirrors the semantics of the server-side matcher at
 * `crates/raisin-rocksdb/src/jobs/handlers/scheduled_trigger.rs::cron_matches`.
 *
 * Supports:
 *   - Special strings: @yearly, @annually, @monthly, @weekly, @daily,
 *     @midnight, @hourly, @every_minute
 *   - 5-field cron: minute hour day month day_of_week
 *   - Per-field: `*`, `* /N` (step), `N-M` (range), `N,M,...` (list), `N`
 *
 * Day-of-week is 1-7 (1=Monday, 7=Sunday) to match the server.
 */

export interface CronValidation {
  valid: boolean
  error?: string
  description?: string
  nextFire?: Date
}

const SPECIAL: Record<string, string> = {
  '@yearly': '0 0 1 1 *',
  '@annually': '0 0 1 1 *',
  '@monthly': '0 0 1 * *',
  '@weekly': '0 0 * * 1',
  '@daily': '0 0 * * *',
  '@midnight': '0 0 * * *',
  '@hourly': '0 * * * *',
  '@every_minute': '* * * * *',
}

const FIELD_RANGES: Array<[number, number]> = [
  [0, 59], // minute
  [0, 23], // hour
  [1, 31], // day
  [1, 12], // month
  [1, 7], // day-of-week (1=Monday)
]

const FIELD_NAMES = ['minute', 'hour', 'day', 'month', 'day-of-week'] as const

interface FieldMatcher {
  matches(value: number): boolean
  values(): number[]
  isWildcard: boolean
}

function expandField(field: string, min: number, max: number, name: string): FieldMatcher {
  if (field === '*') {
    const all: number[] = []
    for (let v = min; v <= max; v++) all.push(v)
    return { matches: () => true, values: () => all, isWildcard: true }
  }

  // Step values: */N
  const stepMatch = field.match(/^\*\/(\d+)$/)
  if (stepMatch) {
    const step = parseInt(stepMatch[1], 10)
    if (step <= 0) throw new Error(`Invalid ${name} step: must be positive`)
    const all: number[] = []
    for (let v = min; v <= max; v++) if (v % step === 0) all.push(v)
    return {
      matches: (v) => v >= min && v <= max && v % step === 0,
      values: () => all,
      isWildcard: false,
    }
  }

  // List: a,b,c
  if (field.includes(',')) {
    const parts = field.split(',').map((p) => p.trim())
    const nums = parts.map((p) => {
      const n = parseInt(p, 10)
      if (isNaN(n) || n < min || n > max) {
        throw new Error(`Invalid ${name} value: ${p} (must be ${min}-${max})`)
      }
      return n
    })
    const set = new Set(nums)
    return { matches: (v) => set.has(v), values: () => [...set].sort((a, b) => a - b), isWildcard: false }
  }

  // Range: a-b
  if (field.includes('-')) {
    const [aStr, bStr] = field.split('-')
    const a = parseInt(aStr, 10)
    const b = parseInt(bStr, 10)
    if (isNaN(a) || isNaN(b) || a < min || b > max || a > b) {
      throw new Error(`Invalid ${name} range: ${field} (must be ${min}-${max})`)
    }
    const all: number[] = []
    for (let v = a; v <= b; v++) all.push(v)
    return { matches: (v) => v >= a && v <= b, values: () => all, isWildcard: false }
  }

  // Simple numeric
  const n = parseInt(field, 10)
  if (isNaN(n) || n < min || n > max) {
    throw new Error(`Invalid ${name} value: ${field} (must be ${min}-${max})`)
  }
  return { matches: (v) => v === n, values: () => [n], isWildcard: false }
}

function parseCron(expr: string): FieldMatcher[] {
  const trimmed = expr.trim()
  const normalized = SPECIAL[trimmed] ?? trimmed
  const fields = normalized.split(/\s+/)
  if (fields.length !== 5) {
    throw new Error(`Expected 5 fields (minute hour day month day-of-week), got ${fields.length}`)
  }
  return fields.map((f, i) => expandField(f, FIELD_RANGES[i][0], FIELD_RANGES[i][1], FIELD_NAMES[i]))
}

function describe(matchers: FieldMatcher[]): string {
  const [m, h, d, mo, dow] = matchers
  // Common short-forms first
  if (m.isWildcard && h.isWildcard && d.isWildcard && mo.isWildcard && dow.isWildcard) {
    return 'Every minute'
  }
  if (m.values().length === 1 && m.values()[0] === 0 && h.isWildcard && d.isWildcard && mo.isWildcard && dow.isWildcard) {
    return 'Every hour at :00'
  }
  if (m.values().length === 1 && h.values().length === 1 && d.isWildcard && mo.isWildcard && dow.isWildcard) {
    const hh = h.values()[0].toString().padStart(2, '0')
    const mm = m.values()[0].toString().padStart(2, '0')
    return `Every day at ${hh}:${mm}`
  }
  // Generic
  const parts: string[] = []
  if (!m.isWildcard) parts.push(`minute(s) ${m.values().join(',')}`)
  if (!h.isWildcard) parts.push(`hour(s) ${h.values().join(',')}`)
  if (!d.isWildcard) parts.push(`day(s) ${d.values().join(',')}`)
  if (!mo.isWildcard) parts.push(`month(s) ${mo.values().join(',')}`)
  if (!dow.isWildcard) {
    const names = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun']
    parts.push(`weekday(s) ${dow.values().map((v) => names[v - 1] ?? v).join(',')}`)
  }
  return parts.length ? `At ${parts.join(', ')}` : 'Every minute'
}

function nextFireAfter(matchers: FieldMatcher[], from: Date): Date | undefined {
  // Brute-force scan ahead minute by minute, up to a year.
  const candidate = new Date(from)
  candidate.setUTCSeconds(0, 0)
  candidate.setUTCMinutes(candidate.getUTCMinutes() + 1)
  const limit = 60 * 24 * 366
  for (let i = 0; i < limit; i++) {
    const minute = candidate.getUTCMinutes()
    const hour = candidate.getUTCHours()
    const day = candidate.getUTCDate()
    const month = candidate.getUTCMonth() + 1
    // getUTCDay: 0=Sunday, 1=Monday, ..., 6=Saturday. Server uses 1=Mon..7=Sun.
    const jsDow = candidate.getUTCDay()
    const dow = jsDow === 0 ? 7 : jsDow
    if (
      matchers[0].matches(minute) &&
      matchers[1].matches(hour) &&
      matchers[2].matches(day) &&
      matchers[3].matches(month) &&
      matchers[4].matches(dow)
    ) {
      return new Date(candidate)
    }
    candidate.setUTCMinutes(candidate.getUTCMinutes() + 1)
  }
  return undefined
}

/**
 * Validate a cron expression and return a human-readable description
 * plus the next fire time after `now`.
 */
export function validateCron(expr: string, now: Date = new Date()): CronValidation {
  if (!expr || !expr.trim()) {
    return { valid: false, error: 'Cron expression is required' }
  }
  try {
    const matchers = parseCron(expr)
    return {
      valid: true,
      description: describe(matchers),
      nextFire: nextFireAfter(matchers, now),
    }
  } catch (e) {
    return { valid: false, error: e instanceof Error ? e.message : 'Invalid cron expression' }
  }
}

/** Common cron expression presets for quick-pick UI. */
export const CRON_PRESETS: ReadonlyArray<{ label: string; expr: string }> = [
  { label: 'Every minute', expr: '* * * * *' },
  { label: 'Every 5 minutes', expr: '*/5 * * * *' },
  { label: 'Every 15 minutes', expr: '*/15 * * * *' },
  { label: 'Every 30 minutes', expr: '*/30 * * * *' },
  { label: 'Every hour', expr: '0 * * * *' },
  { label: 'Every day at midnight', expr: '0 0 * * *' },
  { label: 'Every day at 09:00', expr: '0 9 * * *' },
  { label: 'Every Monday at 09:00', expr: '0 9 * * 1' },
  { label: 'First of month at midnight', expr: '0 0 1 * *' },
]
