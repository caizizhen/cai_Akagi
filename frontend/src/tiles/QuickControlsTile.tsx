import { TileFrame } from '@/components/TileFrame'
import { Button } from '@/components/ui/button'
import { invoke } from '@/lib/tauri'
import { Play, Square, RefreshCw, ListTree, CameraIcon, Activity } from 'lucide-react'
import type { Breakpoint } from '@/tiles/defaults'
import { useState } from 'react'

export function QuickControlsTile({ bp }: { bp: Breakpoint }) {
  const [busy, setBusy] = useState<string | null>(null)

  const call = async (cmd: string) => {
    setBusy(cmd)
    try {
      await invoke(cmd)
    } catch {
      /* errors surface via notify event */
    } finally {
      setBusy(null)
    }
  }

  return (
    <TileFrame id="quick-controls" title="Quick Controls" bp={bp} contentClassName="grid grid-cols-2 gap-1.5">
      <Btn icon={Play}     label="Start Proxy"  onClick={() => call('start_proxy')}    busy={busy === 'start_proxy'} variant="default" />
      <Btn icon={Square}   label="Stop Proxy"   onClick={() => call('stop_proxy')}     busy={busy === 'stop_proxy'} variant="destructive" />
      <Btn icon={RefreshCw} label="Reconnect"    onClick={() => call('start_proxy')} busy={busy === 'reconnect'} variant="outline" />
      <Btn icon={ListTree} label="List Bots"    onClick={() => call('list_bots')}    busy={busy === 'list_bots'} variant="outline" />
      <Btn icon={CameraIcon} label="Snapshot"   onClick={() => call('get_game_snapshot')} busy={busy === 'snapshot'} variant="outline" />
      <Btn icon={Activity} label="Analysis"     onClick={() => call('get_analysis')} busy={busy === 'analysis'} variant="outline" />
    </TileFrame>
  )
}

function Btn({
  icon: Icon, label, onClick, busy, variant,
}: {
  icon: typeof Play
  label: string
  onClick: () => void
  busy: boolean
  variant: 'default' | 'destructive' | 'outline'
}) {
  return (
    <Button
      variant={variant}
      size="sm"
      onPointerDown={(e) => e.stopPropagation()}
      onClick={onClick}
      disabled={busy}
      className="justify-start gap-2 text-xs"
    >
      <Icon className="h-3.5 w-3.5" />
      <span>{label}</span>
    </Button>
  )
}
