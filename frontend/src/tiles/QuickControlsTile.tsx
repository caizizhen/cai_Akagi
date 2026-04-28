import { TileFrame } from '@/components/TileFrame'
import { Button } from '@/components/ui/button'
import { invoke } from '@/lib/tauri'
import { Play, Square, RefreshCw, ListTree, Camera, Activity } from 'lucide-react'
import type { Breakpoint } from '@/tiles/defaults'
import { useState } from 'react'
import { useGameStore } from '@/stores/gameStore'
import { useAnalysisStore } from '@/stores/analysisStore'
import { useBotStore } from '@/stores/botStore'
import type {
  AnalysisResult,
  BotInfo,
  GameStateSnapshot,
  MahgenView,
} from '@/types'

type Action = {
  id: string
  label: string
  icon: typeof Play
  variant: 'default' | 'destructive' | 'outline'
  run: () => Promise<unknown>
}

export function QuickControlsTile({ bp }: { bp: Breakpoint }) {
  const [busy, setBusy] = useState<string | null>(null)

  const refreshGame = async () => {
    const [snap, view] = await Promise.all([
      invoke<GameStateSnapshot | null>('get_game_snapshot'),
      invoke<MahgenView | null>('get_mahgen_view'),
    ])
    useGameStore.getState().setGame(snap)
    useGameStore.getState().setView(view)
  }

  const actions: Action[] = [
    {
      id: 'start',
      label: 'Start Proxy',
      icon: Play,
      variant: 'default',
      run: () => invoke('start_proxy'),
    },
    {
      id: 'stop',
      label: 'Stop Proxy',
      icon: Square,
      variant: 'destructive',
      run: () => invoke('stop_proxy'),
    },
    {
      id: 'reconnect',
      label: 'Reconnect',
      icon: RefreshCw,
      variant: 'outline',
      run: async () => {
        // best-effort stop, then start; ignore "not running" error from stop.
        await invoke('stop_proxy').catch(() => {})
        await invoke('start_proxy')
      },
    },
    {
      id: 'list-bots',
      label: 'List Bots',
      icon: ListTree,
      variant: 'outline',
      run: async () => {
        const bots = await invoke<BotInfo[]>('list_bots')
        useBotStore.getState().setList(bots)
      },
    },
    {
      id: 'snapshot',
      label: 'Refresh Game',
      icon: Camera,
      variant: 'outline',
      run: refreshGame,
    },
    {
      id: 'analysis',
      label: 'Refresh Analysis',
      icon: Activity,
      variant: 'outline',
      run: async () => {
        const a = await invoke<AnalysisResult | null>('get_analysis')
        useAnalysisStore.getState().set(a)
      },
    },
  ]

  const call = async (action: Action) => {
    setBusy(action.id)
    try {
      await action.run()
    } catch {
      /* surfaced via notify event */
    } finally {
      setBusy(null)
    }
  }

  return (
    <TileFrame id="quick-controls" title="Quick Controls" bp={bp} contentClassName="grid grid-cols-2 gap-1.5">
      {actions.map((a) => (
        <Button
          key={a.id}
          variant={a.variant}
          size="sm"
          onPointerDown={(e) => e.stopPropagation()}
          onClick={() => call(a)}
          disabled={busy === a.id}
          className="justify-start gap-2 text-xs"
        >
          <a.icon className="h-3.5 w-3.5" />
          <span>{a.label}</span>
        </Button>
      ))}
    </TileFrame>
  )
}
