import { TileFrame } from '@/components/TileFrame'
import { Mahgen } from '@/components/Mahgen'
import { useGameStore } from '@/stores/gameStore'
import { fmtScore, relativeKind, BAKAZE_LABEL } from '@/lib/format'
import type { Breakpoint, TileId } from '@/tiles/defaults'

const SEAT_TO_TILE: Record<number, TileId> = {
  0: 'player-0',
  1: 'player-1',
  2: 'player-2',
  3: 'player-3',
}

const SEAT_TITLE: Record<number, string> = {
  0: 'Player 1',
  1: 'Player 2',
  2: 'Player 3',
  3: 'Player 4',
}

const KIND_LABEL: Record<string, string> = {
  self: 'Self',
  shimocha: '下家',
  toimen: '対面',
  kamicha: '上家',
}

export function PlayerTile({ seat, bp }: { seat: 0 | 1 | 2 | 3; bp: Breakpoint }) {
  const game = useGameStore((s) => s.game)
  const view = useGameStore((s) => s.view)
  const player = game?.players[seat]
  const playerView = view?.players[seat]
  const ourSeat = game?.our_seat ?? null
  const kind = relativeKind(seat, ourSeat)
  const isSelf = kind === 'self'
  const id = SEAT_TO_TILE[seat]
  const title = `${SEAT_TITLE[seat]}${isSelf ? ' (Self)' : ''}`

  // bakaze of this seat (E/S/W/N rotates from oya)
  const seatWind = game ? bakazeFor(seat, game.oya) : '—'

  return (
    <TileFrame
      id={id}
      title={title}
      bp={bp}
      rightSlot={
        <span className="text-[10px] uppercase tracking-wider text-muted-foreground px-1">
          {KIND_LABEL[kind]}
        </span>
      }
      contentClassName="flex flex-col gap-2"
    >
      <div className="flex items-baseline justify-between">
        <div className="flex items-center gap-2">
          <span className="text-sm font-semibold">{seatWind}</span>
          {player?.riichi_declared && (
            <span className="rounded bg-amber-500/15 text-amber-400 px-1.5 py-0.5 text-[10px] font-semibold tracking-wider uppercase">
              Riichi
            </span>
          )}
        </div>
        <span className="font-mono text-base font-semibold">{fmtScore(player?.score)}</span>
      </div>

      {playerView && (
        <>
          {isSelf ? (
            <div className="flex flex-wrap items-end gap-1">
              <Mahgen seq={playerView.hand} kind="hand" />
            </div>
          ) : (
            <div className="flex flex-wrap items-end gap-1">
              <Mahgen seq={playerView.hand} kind="melds" />
            </div>
          )}

          {playerView.melds.length > 0 && (
            <div className="flex flex-wrap items-end gap-1">
              {playerView.melds.map((meld, i) => (
                <Mahgen key={i} seq={meld} kind="melds" />
              ))}
            </div>
          )}

          {playerView.river && (
            <div className="mt-auto">
              <Mahgen seq={playerView.river} kind="river" riverMode />
            </div>
          )}
        </>
      )}
    </TileFrame>
  )
}

// East = oya seat. Each subsequent seat rotates S → W → N.
function bakazeFor(seat: number, oya: number): string {
  const order = ['E', 'S', 'W', 'N']
  return BAKAZE_LABEL[order[(seat - oya + 4) % 4]]
}
