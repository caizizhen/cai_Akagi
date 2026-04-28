import { TileFrame } from '@/components/TileFrame'
import { useAnalysisStore } from '@/stores/analysisStore'
import { useGameStore } from '@/stores/gameStore'
import { pct } from '@/lib/format'
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table'
import type { OpponentRisk } from '@/types'
import type { Breakpoint } from '@/tiles/defaults'

// Stable empty fallback — selector must not allocate a new array per render.
// Zustand v5 uses Object.is on the selector output and would loop forever.
const NO_OPPONENTS: readonly OpponentRisk[] = []

export function OpponentsTile({ bp }: { bp: Breakpoint }) {
  const opponents = useAnalysisStore((s) => s.result?.opponents ?? NO_OPPONENTS)
  const ourSeat = useGameStore((s) => s.game?.our_seat ?? null)

  return (
    <TileFrame id="opponents" title="Opponents" bp={bp} contentClassName="p-0">
      {opponents.length === 0 ? (
        <span className="text-muted-foreground text-sm p-3 block">Awaiting analysis.</span>
      ) : (
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead className="text-[10px] uppercase">Seat</TableHead>
              <TableHead className="text-[10px] uppercase">Tenpai</TableHead>
              <TableHead className="text-[10px] uppercase">Riichi</TableHead>
              <TableHead className="text-[10px] uppercase text-right">Max Risk</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {opponents.map((o) => {
              const maxRisk = o.risk.length ? Math.max(...o.risk) : 0
              return (
                <TableRow key={o.seat}>
                  <TableCell className="font-mono">{seatLabel(o.seat, ourSeat)}</TableCell>
                  <TableCell className="font-mono">{pct(o.tenpai_rate)}</TableCell>
                  <TableCell>{o.is_riichi ? '●' : '—'}</TableCell>
                  <TableCell className="font-mono text-right">{pct(maxRisk)}</TableCell>
                </TableRow>
              )
            })}
          </TableBody>
        </Table>
      )}
    </TileFrame>
  )
}

function seatLabel(seat: number, ourSeat: number | null): string {
  if (ourSeat == null) return String(seat)
  const d = (seat - ourSeat + 4) % 4
  return d === 1 ? '下' : d === 2 ? '対' : d === 3 ? '上' : '自'
}
