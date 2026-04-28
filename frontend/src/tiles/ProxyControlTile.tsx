import { TileFrame } from '@/components/TileFrame'
import { Button } from '@/components/ui/button'
import { useProxyStore } from '@/stores/proxyStore'
import { invoke } from '@/lib/tauri'
import { Play, Square } from 'lucide-react'
import type { Breakpoint } from '@/tiles/defaults'
import { useState } from 'react'

const STATE_COLOR: Record<string, string> = {
  running:  'bg-emerald-500',
  starting: 'bg-amber-500',
  stopped:  'bg-zinc-500',
  error:    'bg-red-500',
}

export function ProxyControlTile({ bp }: { bp: Breakpoint }) {
  const status = useProxyStore((s) => s.status)
  const [busy, setBusy] = useState(false)

  const dot = STATE_COLOR[status.state] ?? 'bg-zinc-500'
  const addr = 'addr' in status && typeof status.addr === 'string' ? status.addr : '—'

  const call = async (cmd: 'start_proxy' | 'stop_proxy') => {
    setBusy(true)
    try {
      await invoke(cmd)
    } catch {
      /* surfaced via notify */
    } finally {
      setBusy(false)
    }
  }

  return (
    <TileFrame id="proxy-control" title="Proxy" bp={bp} contentClassName="flex flex-col gap-2">
      <div className="flex items-center gap-2">
        <span className={`h-2 w-2 rounded-full ${dot}`} />
        <span className="text-sm font-medium capitalize">{status.state}</span>
        <span className="text-xs font-mono text-muted-foreground ml-auto">{addr}</span>
      </div>

      <div className="flex gap-1.5">
        <Button
          variant="default"
          size="sm"
          className="flex-1 gap-1.5"
          onPointerDown={(e) => e.stopPropagation()}
          onClick={() => call('start_proxy')}
          disabled={busy || status.state === 'running' || status.state === 'starting'}
        >
          <Play className="h-3.5 w-3.5" />
          Start
        </Button>
        <Button
          variant="destructive"
          size="sm"
          className="flex-1 gap-1.5"
          onPointerDown={(e) => e.stopPropagation()}
          onClick={() => call('stop_proxy')}
          disabled={busy || status.state === 'stopped'}
        >
          <Square className="h-3.5 w-3.5" />
          Stop
        </Button>
      </div>

      {status.state === 'error' && 'error' in status && (
        <span className="text-[11px] text-red-400 font-mono">{status.error}</span>
      )}
    </TileFrame>
  )
}
