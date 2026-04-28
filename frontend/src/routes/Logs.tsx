import { FolderOpen, RefreshCw } from 'lucide-react'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { invoke } from '@/lib/tauri'
import { useConfigStore } from '@/stores/configStore'
import { useState } from 'react'

export function Logs() {
  const logDir = useConfigStore((s) => s.logDir)
  const setLogDir = useConfigStore((s) => s.setLogDir)
  const [busy, setBusy] = useState(false)

  const refresh = async () => {
    setBusy(true)
    try {
      const dir = await invoke<string>('get_log_dir')
      setLogDir(dir)
    } catch {
      /* noop */
    } finally {
      setBusy(false)
    }
  }

  const openFolder = async () => {
    try {
      await invoke('open_log_folder')
    } catch {
      /* noop */
    }
  }

  return (
    <div className="p-6 flex flex-col gap-4 w-full">
      <header className="flex items-center justify-between">
        <h1 className="text-2xl font-semibold">Logs</h1>
        <div className="flex gap-2">
          <Button variant="outline" size="sm" onClick={refresh} disabled={busy} className="gap-1.5">
            <RefreshCw className={`h-4 w-4 ${busy ? 'animate-spin' : ''}`} />
            Refresh
          </Button>
          <Button size="sm" onClick={openFolder} className="gap-1.5">
            <FolderOpen className="h-4 w-4" />
            Open log folder
          </Button>
        </div>
      </header>

      <Card>
        <CardHeader>
          <CardTitle>Active session</CardTitle>
        </CardHeader>
        <CardContent>
          <div className="font-mono text-sm break-all text-muted-foreground">
            {logDir || '— no session —'}
          </div>
        </CardContent>
      </Card>

      <p className="text-sm text-muted-foreground">
        Tail viewer is on the roadmap. For now, open the log folder in your file manager.
      </p>
    </div>
  )
}
