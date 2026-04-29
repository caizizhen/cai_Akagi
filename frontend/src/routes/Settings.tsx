import { useEffect, useState } from 'react'
import { useBlocker } from 'react-router-dom'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
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
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { invoke } from '@/lib/tauri'
import { useConfigStore } from '@/stores/configStore'
import type { AppConfig } from '@/types'

export function Settings() {
  const stored = useConfigStore((s) => s.config)
  const setStored = useConfigStore((s) => s.setConfig)
  const [draft, setDraft] = useState<AppConfig | null>(stored)
  const [saving, setSaving] = useState(false)
  const [err, setErr] = useState<string | null>(null)

  useEffect(() => {
    if (stored) setDraft(stored)
  }, [stored])

  useEffect(() => {
    if (!stored) {
      invoke<AppConfig>('get_config').then(setStored).catch(() => {})
    }
  }, [stored, setStored])

  const dirty = !!draft && !!stored && JSON.stringify(draft) !== JSON.stringify(stored)

  const blocker = useBlocker(
    ({ currentLocation, nextLocation }) =>
      dirty && currentLocation.pathname !== nextLocation.pathname,
  )

  useEffect(() => {
    if (!dirty) return
    const handler = (e: BeforeUnloadEvent) => {
      e.preventDefault()
      e.returnValue = ''
    }
    window.addEventListener('beforeunload', handler)
    return () => window.removeEventListener('beforeunload', handler)
  }, [dirty])

  if (!draft) {
    return <div className="p-6 text-muted-foreground">Loading config…</div>
  }

  const save = async () => {
    setSaving(true)
    setErr(null)
    try {
      await invoke('update_config', { newConfig: draft })
      setStored(draft)
    } catch (e) {
      setErr(String(e))
    } finally {
      setSaving(false)
    }
  }

  const saveAndLeave = async () => {
    setSaving(true)
    setErr(null)
    try {
      await invoke('update_config', { newConfig: draft })
      setStored(draft)
      blocker.proceed?.()
    } catch (e) {
      setErr(String(e))
      blocker.reset?.()
    } finally {
      setSaving(false)
    }
  }

  const discardAndLeave = () => {
    setDraft(stored)
    blocker.proceed?.()
  }

  return (
    <div className="p-6 w-full flex flex-col gap-6">
      <header className="flex items-center justify-between">
        <h1 className="text-2xl font-semibold">Settings</h1>
        <div className="flex gap-2">
          <Button variant="outline" onClick={() => setDraft(stored)} disabled={!dirty || saving}>
            Reset
          </Button>
          <Button onClick={save} disabled={!dirty || saving}>
            {saving ? 'Saving…' : 'Save'}
          </Button>
        </div>
      </header>

      {err && (
        <div className="rounded-md border border-red-500/40 bg-red-500/10 px-3 py-2 text-sm text-red-400">
          {err}
        </div>
      )}

      <Card>
        <CardHeader>
          <CardTitle>General</CardTitle>
        </CardHeader>
        <CardContent className="grid gap-4">
          <Field label="Language">
            <Select
              value={draft.general.language}
              onValueChange={(v) => setDraft({ ...draft, general: { ...draft.general, language: v } })}
            >
              <SelectTrigger className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="zh-TW">繁體中文</SelectItem>
                <SelectItem value="zh-CN">简体中文</SelectItem>
                <SelectItem value="ja">日本語</SelectItem>
                <SelectItem value="en">English</SelectItem>
              </SelectContent>
            </Select>
          </Field>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Proxy</CardTitle>
        </CardHeader>
        <CardContent className="grid gap-4">
          <Toggle
            label="Enabled"
            value={draft.proxy.enabled}
            onChange={(v) => setDraft({ ...draft, proxy: { ...draft.proxy, enabled: v } })}
          />
          <Field label="Address">
            <Input
              value={draft.proxy.addr}
              onChange={(e) => setDraft({ ...draft, proxy: { ...draft.proxy, addr: e.target.value } })}
              placeholder="127.0.0.1:11656"
            />
          </Field>
          <Field label="CA directory" hint="Where root certificate / keys are written.">
            <Input
              value={draft.proxy.ca_dir}
              onChange={(e) => setDraft({ ...draft, proxy: { ...draft.proxy, ca_dir: e.target.value } })}
            />
          </Field>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Logging</CardTitle>
        </CardHeader>
        <CardContent className="grid gap-4">
          <Field label="Directory">
            <Input
              value={draft.logging.dir}
              onChange={(e) => setDraft({ ...draft, logging: { ...draft.logging, dir: e.target.value } })}
            />
          </Field>
          <Field label="App log level">
            <Input
              value={draft.logging.level}
              onChange={(e) => setDraft({ ...draft, logging: { ...draft.logging, level: e.target.value } })}
              placeholder="info"
            />
          </Field>
          <Field label="Crate log level">
            <Input
              value={draft.logging.all_level}
              onChange={(e) => setDraft({ ...draft, logging: { ...draft.logging, all_level: e.target.value } })}
              placeholder="warn"
            />
          </Field>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Bots</CardTitle>
        </CardHeader>
        <CardContent className="grid gap-4">
          <Toggle
            label="Bot enabled"
            value={draft.bot.enabled}
            onChange={(v) => setDraft({ ...draft, bot: { ...draft.bot, enabled: v } })}
          />
          <Toggle
            label="Auto-sync (uv sync on first spawn)"
            value={draft.bot.auto_sync}
            onChange={(v) => setDraft({ ...draft, bot: { ...draft.bot, auto_sync: v } })}
          />
          <Field label="Active bot (4p)">
            <Input
              value={draft.bot.active_4p}
              onChange={(e) => setDraft({ ...draft, bot: { ...draft.bot, active_4p: e.target.value } })}
              placeholder="mortal"
            />
          </Field>
          <Field label="Active bot (3p, sanma)">
            <Input
              value={draft.bot.active_3p}
              onChange={(e) => setDraft({ ...draft, bot: { ...draft.bot, active_3p: e.target.value } })}
              placeholder="(none)"
            />
          </Field>
          <Field label="Bot directory">
            <Input
              value={draft.bot.dir}
              onChange={(e) => setDraft({ ...draft, bot: { ...draft.bot, dir: e.target.value } })}
            />
          </Field>
        </CardContent>
      </Card>

      <Dialog
        open={blocker.state === 'blocked'}
        onOpenChange={(open) => {
          if (!open) blocker.reset?.()
        }}
      >
        <DialogContent showCloseButton={false}>
          <DialogHeader>
            <DialogTitle>Unsaved changes</DialogTitle>
            <DialogDescription>
              You have unsaved settings changes. Save them before leaving, discard them, or stay on this page.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter className="bg-transparent p-0 border-0 mx-0 mb-0">
            <Button variant="outline" size="sm" onClick={() => blocker.reset?.()} disabled={saving}>
              Stay
            </Button>
            <Button variant="destructive" size="sm" onClick={discardAndLeave} disabled={saving}>
              Discard
            </Button>
            <Button size="sm" onClick={saveAndLeave} disabled={saving}>
              {saving ? 'Saving…' : 'Save & leave'}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  )
}

function Field({ label, hint, children }: { label: string; hint?: string; children: React.ReactNode }) {
  return (
    <div className="grid gap-1.5">
      <Label>{label}</Label>
      {children}
      {hint && <span className="text-xs text-muted-foreground">{hint}</span>}
    </div>
  )
}

function Toggle({ label, value, onChange }: { label: string; value: boolean; onChange: (v: boolean) => void }) {
  return (
    <div className="flex items-center justify-between">
      <Label>{label}</Label>
      <Switch checked={value} onCheckedChange={onChange} />
    </div>
  )
}
