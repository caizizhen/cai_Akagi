import { TileFrame } from '@/components/TileFrame'
import { Mahgen } from '@/components/Mahgen'
import { useGameStore } from '@/stores/gameStore'
import { kyokuLabel } from '@/lib/format'
import type { Breakpoint } from '@/tiles/defaults'

export function HeaderTile({ bp }: { bp: Breakpoint }) {
  const game = useGameStore((s) => s.game)
  const view = useGameStore((s) => s.view)

  return (
    <TileFrame id="header" title="Game" bp={bp} contentClassName="flex items-center gap-6 px-4">
      <div className="flex items-center gap-2">
        <span className="inline-flex items-center gap-1.5 rounded-full bg-emerald-500/15 text-emerald-400 px-2 py-0.5 text-[10px] font-semibold tracking-wider uppercase">
          <span className="h-1.5 w-1.5 rounded-full bg-emerald-400" />
          Live
        </span>
        <h2 className="text-lg font-semibold">
          {game ? kyokuLabel(game.bakaze, game.kyoku) : '—'}
        </h2>
      </div>

      {view?.dora_indicators && (
        <div className="flex items-center gap-2 rounded-md border border-border px-2 py-1">
          <span className="text-[10px] uppercase tracking-wider text-muted-foreground">Dora</span>
          <Mahgen seq={view.dora_indicators} kind="dora" />
        </div>
      )}

      <Stat label="Honba"   value={game?.honba ?? 0} />
      <Stat label="Kyotaku" value={game?.kyotaku ?? 0} />
      <Stat label="Turn"    value={game?.turn_count ?? 0} />
      {game && (
        <Stat
          label="Phase"
          value={game.phase}
          mono
        />
      )}
    </TileFrame>
  )
}

function Stat({ label, value, mono }: { label: string; value: number | string; mono?: boolean }) {
  return (
    <div className="flex flex-col">
      <span className="text-[10px] uppercase tracking-wider text-muted-foreground">{label}</span>
      <span className={mono ? 'font-mono text-sm' : 'font-medium text-sm'}>{value}</span>
    </div>
  )
}
