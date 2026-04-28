export function fmtScore(n: number | null | undefined): string {
  return new Intl.NumberFormat('en-US').format(n ?? 0)
}

export function fmtTime(d: Date = new Date()): string {
  const h = String(d.getHours()).padStart(2, '0')
  const m = String(d.getMinutes()).padStart(2, '0')
  const s = String(d.getSeconds()).padStart(2, '0')
  return `${h}:${m}:${s}`
}

export function pct(v: number | null | undefined, digits = 1): string {
  if (v == null || Number.isNaN(v)) return '—'
  return `${Number(v).toFixed(digits)}%`
}

export function riskClass(v: number | null | undefined): string {
  if (v == null) return ''
  if (v >= 20) return 'risk-high'
  if (v >= 10) return 'risk-mid'
  return 'risk-low'
}

export type RelativeKind = 'self' | 'shimocha' | 'toimen' | 'kamicha'

export function relativeKind(seat: number, ourSeat: number | null): RelativeKind {
  if (ourSeat == null) return 'self'
  const d = (seat - ourSeat + 4) % 4
  return d === 0 ? 'self' : d === 1 ? 'shimocha' : d === 2 ? 'toimen' : 'kamicha'
}

export const BAKAZE_LABEL: Record<string, string> = {
  E: '東',
  S: '南',
  W: '西',
  N: '北',
}

export function kyokuLabel(bakaze: string, kyoku: number): string {
  return `${BAKAZE_LABEL[bakaze] ?? bakaze} ${kyoku}局`
}
