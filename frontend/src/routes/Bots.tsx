import { useEffect, useState } from 'react'
import { Plus, Settings as SettingsIcon, RefreshCw, CheckCircle2, Trash2 } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Switch } from '@/components/ui/switch'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from '@/components/ui/dialog'
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from '@/components/ui/sheet'
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table'
import { invoke } from '@/lib/tauri'
import { useBotStore } from '@/stores/botStore'
import { useConfigStore } from '@/stores/configStore'
import type { AppConfig, BotInfo, BotSettings, FieldSpec } from '@/types'

export function Bots() {
  const list = useBotStore((s) => s.list)
  const setList = useBotStore((s) => s.setList)
  const config = useConfigStore((s) => s.config)
  const setConfig = useConfigStore((s) => s.setConfig)
  const [loading, setLoading] = useState(false)
  const [editing, setEditing] = useState<string | null>(null)
  const [deleting, setDeleting] = useState<string | null>(null)

  const refresh = async () => {
    setLoading(true)
    try {
      const [bots, cfg] = await Promise.all([
        invoke<BotInfo[]>('list_bots'),
        invoke<AppConfig>('get_config'),
      ])
      setList(bots)
      setConfig(cfg)
    } catch {
      /* notify event will surface */
    } finally {
      setLoading(false)
    }
  }

  useEffect(() => {
    if (list.length === 0) void refresh()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  const setActive = async (name: string) => {
    if (config?.bot.active === name) return
    // Optimistic: flip immediately so the Switch reflects the click; refresh
    // backfills from backend in case the call fails or the value differs.
    if (config) setConfig({ ...config, bot: { ...config.bot, active: name } })
    try {
      await invoke('set_active_bot', { name })
    } catch {
      /* noop */
    } finally {
      void refresh()
    }
  }

  return (
    <div className="p-6 flex flex-col gap-4">
      <header className="flex items-center justify-between">
        <h1 className="text-2xl font-semibold">Bots</h1>
        <div className="flex gap-2">
          <Button variant="outline" size="sm" onClick={refresh} disabled={loading} className="gap-1.5">
            <RefreshCw className={`h-4 w-4 ${loading ? 'animate-spin' : ''}`} />
            Refresh
          </Button>
          <InstallFromGithubDialog onInstalled={refresh} />
        </div>
      </header>

      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>Name</TableHead>
            <TableHead>Version</TableHead>
            <TableHead>Manifest</TableHead>
            <TableHead>Active</TableHead>
            <TableHead className="text-right">Actions</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {list.length === 0 ? (
            <TableRow>
              <TableCell colSpan={5} className="text-center text-muted-foreground">
                {loading ? 'Loading…' : 'No bots installed.'}
              </TableCell>
            </TableRow>
          ) : list.map((bot) => (
            <TableRow key={bot.name}>
              <TableCell>
                <div className="flex flex-col">
                  <span className="font-medium">{bot.manifest?.bot.display ?? bot.name}</span>
                  <span className="text-xs text-muted-foreground font-mono">{bot.dir}</span>
                </div>
              </TableCell>
              <TableCell className="font-mono text-xs">{bot.manifest?.bot.version ?? '—'}</TableCell>
              <TableCell>{bot.manifest ? <CheckCircle2 className="h-4 w-4 text-emerald-400" /> : '—'}</TableCell>
              <TableCell>
                <Switch
                  checked={config?.bot.active === bot.name}
                  onCheckedChange={(v) => v && void setActive(bot.name)}
                />
              </TableCell>
              <TableCell className="text-right">
                <div className="flex items-center justify-end gap-1">
                  <Button
                    size="sm"
                    variant="ghost"
                    onClick={() => setEditing(bot.name)}
                    disabled={!bot.manifest}
                    className="gap-1.5"
                  >
                    <SettingsIcon className="h-4 w-4" />
                    Configure
                  </Button>
                  <Button
                    size="sm"
                    variant="ghost"
                    onClick={() => setDeleting(bot.name)}
                    disabled={config?.bot.active === bot.name}
                    title={config?.bot.active === bot.name ? 'Switch active bot first' : 'Delete'}
                    className="gap-1.5 text-red-400 hover:text-red-400"
                  >
                    <Trash2 className="h-4 w-4" />
                  </Button>
                </div>
              </TableCell>
            </TableRow>
          ))}
        </TableBody>
      </Table>

      {editing && (
        <BotSettingsDrawer
          name={editing}
          open
          onOpenChange={(open) => !open && setEditing(null)}
        />
      )}

      {deleting && (
        <DeleteBotDialog
          name={deleting}
          onClose={() => setDeleting(null)}
          onDeleted={refresh}
        />
      )}
    </div>
  )
}

function DeleteBotDialog({
  name, onClose, onDeleted,
}: {
  name: string
  onClose: () => void
  onDeleted: () => void
}) {
  const [busy, setBusy] = useState(false)
  const [err, setErr] = useState<string | null>(null)

  const submit = async () => {
    setBusy(true)
    setErr(null)
    try {
      await invoke('delete_bot', { name })
      onClose()
      onDeleted()
    } catch (e) {
      setErr(String(e))
    } finally {
      setBusy(false)
    }
  }

  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Delete bot {name}?</DialogTitle>
        </DialogHeader>
        <p className="text-sm text-muted-foreground">
          This permanently removes <span className="font-mono">{name}</span> from
          the bots directory, including its installed files, virtualenv, and
          settings. This action cannot be undone.
        </p>
        {err && <span className="text-sm text-red-400">{err}</span>}
        <DialogFooter>
          <Button variant="outline" onClick={onClose}>Cancel</Button>
          <Button variant="destructive" onClick={submit} disabled={busy}>
            {busy ? 'Deleting…' : 'Delete'}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

function InstallFromGithubDialog({ onInstalled }: { onInstalled: () => void }) {
  const [open, setOpen] = useState(false)
  const [repo, setRepo] = useState('')
  const [name, setName] = useState('')
  const [glob, setGlob] = useState('')
  const [busy, setBusy] = useState(false)
  const [err, setErr] = useState<string | null>(null)

  const submit = async () => {
    setBusy(true)
    setErr(null)
    try {
      await invoke('install_bot_from_github', {
        repo,
        assetGlob: glob || undefined,
        name: name || undefined,
      })
      setOpen(false)
      setRepo('')
      setName('')
      setGlob('')
      onInstalled()
    } catch (e) {
      setErr(String(e))
    } finally {
      setBusy(false)
    }
  }

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogTrigger asChild>
        <Button size="sm" className="gap-1.5">
          <Plus className="h-4 w-4" />
          Install from GitHub
        </Button>
      </DialogTrigger>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Install bot from GitHub release</DialogTitle>
        </DialogHeader>
        <div className="grid gap-4 py-2">
          <div className="grid gap-1.5">
            <Label>Repo</Label>
            <Input
              value={repo}
              onChange={(e) => setRepo(e.target.value)}
              placeholder="user/mortal-bot"
            />
            <span className="text-xs text-muted-foreground">
              Accepts <span className="font-mono">owner/name</span> or a full
              GitHub URL (<span className="font-mono">https://github.com/owner/name</span>).
            </span>
          </div>
          <div className="grid gap-1.5">
            <Label>Asset glob (optional)</Label>
            <Input value={glob} onChange={(e) => setGlob(e.target.value)} placeholder="*-linux.zip" />
          </div>
          <div className="grid gap-1.5">
            <Label>Local name (optional, defaults to repo name)</Label>
            <Input value={name} onChange={(e) => setName(e.target.value)} placeholder="mortal" />
          </div>
          {err && (
            <span className="text-sm text-red-400">{err}</span>
          )}
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={() => setOpen(false)}>Cancel</Button>
          <Button onClick={submit} disabled={busy || !repo}>
            {busy ? 'Installing…' : 'Install'}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

function BotSettingsDrawer({ name, open, onOpenChange }: { name: string; open: boolean; onOpenChange: (open: boolean) => void }) {
  const [data, setData] = useState<BotSettings | null>(null)
  const [values, setValues] = useState<Record<string, unknown>>({})
  const [saving, setSaving] = useState(false)
  const [err, setErr] = useState<string | null>(null)

  useEffect(() => {
    if (!open) return
    invoke<BotSettings>('get_bot_settings', { name })
      .then((s) => {
        setData(s)
        setValues(s.values)
      })
      .catch((e) => setErr(String(e)))
  }, [name, open])

  const save = async () => {
    setSaving(true)
    setErr(null)
    try {
      await invoke('update_bot_settings', { name, values })
      onOpenChange(false)
    } catch (e) {
      setErr(String(e))
    } finally {
      setSaving(false)
    }
  }

  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent className="flex flex-col gap-4 overflow-y-auto p-4 sm:max-w-md">
        <SheetHeader className="p-0">
          <SheetTitle>{data?.manifest.bot.display ?? name}</SheetTitle>
          <SheetDescription>{data?.manifest.bot.description}</SheetDescription>
        </SheetHeader>

        {!data ? (
          <div className="text-muted-foreground text-sm">Loading…</div>
        ) : (
          <div className="grid gap-4">
            {Object.entries(data.manifest.settings).map(([key, spec]) => (
              <ManifestField
                key={key}
                fieldKey={key}
                spec={spec}
                value={values[key] ?? spec.default}
                onChange={(v) => setValues({ ...values, [key]: v })}
              />
            ))}
          </div>
        )}

        {err && <span className="text-sm text-red-400">{err}</span>}

        <div className="flex justify-end gap-2 mt-auto pt-2 border-t border-border">
          <Button variant="outline" onClick={() => onOpenChange(false)}>Cancel</Button>
          <Button onClick={save} disabled={saving || !data}>
            {saving ? 'Saving…' : 'Save'}
          </Button>
        </div>
      </SheetContent>
    </Sheet>
  )
}

function ManifestField({
  fieldKey, spec, value, onChange,
}: {
  fieldKey: string
  spec: FieldSpec
  value: unknown
  onChange: (v: unknown) => void
}) {
  return (
    <div className="grid gap-1.5">
      <Label>{spec.label}</Label>
      {renderInput(fieldKey, spec, value, onChange)}
      {spec.help && <span className="text-xs text-muted-foreground">{spec.help}</span>}
    </div>
  )
}

function renderInput(_key: string, spec: FieldSpec, value: unknown, onChange: (v: unknown) => void) {
  switch (spec.type) {
    case 'bool':
      return <Switch checked={Boolean(value)} onCheckedChange={onChange} />
    case 'enum':
      return (
        <Select value={String(value ?? '')} onValueChange={onChange}>
          <SelectTrigger><SelectValue /></SelectTrigger>
          <SelectContent>
            {(spec.choices ?? []).map((c) => (
              <SelectItem key={c} value={c}>{c}</SelectItem>
            ))}
          </SelectContent>
        </Select>
      )
    case 'int':
    case 'float':
      return (
        <Input
          type="number"
          value={value == null ? '' : String(value)}
          min={spec.min}
          max={spec.max}
          step={spec.step ?? (spec.type === 'int' ? 1 : 'any')}
          onChange={(e) => {
            const v = e.target.value
            if (v === '') onChange(null)
            else onChange(spec.type === 'int' ? parseInt(v, 10) : parseFloat(v))
          }}
        />
      )
    default:
      return (
        <Input
          type={spec.secret ? 'password' : 'text'}
          value={String(value ?? '')}
          onChange={(e) => onChange(e.target.value)}
        />
      )
  }
}
