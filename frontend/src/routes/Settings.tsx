import { useEffect, useState } from 'react'
import { Link, useBlocker } from 'react-router-dom'
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
import type { AppConfig, CaptureMode, DetectedBrowser } from '@/types'

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
          <Button variant="ghost" asChild>
            <Link to="/setup?rerun=1">Re-run setup</Link>
          </Button>
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

      <CaptureCard draft={draft} setDraft={setDraft} />

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

function CaptureCard({
  draft,
  setDraft,
}: {
  draft: AppConfig
  setDraft: (c: AppConfig) => void
}) {
  const mode: CaptureMode = draft.capture?.mode ?? 'mitm'
  const chromium = draft.capture?.chromium ?? {
    executable: '',
    user_data_dir: '',
    start_url: 'https://game.maj-soul.com/1/',
    cft_channel: 'stable',
    force_cft: false,
    extra_args: [],
  }
  const [detected, setDetected] = useState<DetectedBrowser[] | null>(null)
  const [detecting, setDetecting] = useState(false)

  const probe = async () => {
    setDetecting(true)
    try {
      const list = await invoke<DetectedBrowser[]>('detect_system_chrome')
      setDetected(list)
    } catch {
      setDetected([])
    } finally {
      setDetecting(false)
    }
  }

  useEffect(() => {
    if (mode === 'chromium' && detected === null) {
      probe()
    }
  }, [mode]) // eslint-disable-line react-hooks/exhaustive-deps

  const setMode = (v: CaptureMode) =>
    setDraft({
      ...draft,
      capture: {
        mode: v,
        chromium,
      },
    })
  const setChromium = (patch: Partial<typeof chromium>) =>
    setDraft({
      ...draft,
      capture: {
        mode,
        chromium: { ...chromium, ...patch },
      },
    })

  return (
    <Card>
      <CardHeader>
        <CardTitle>Capture</CardTitle>
      </CardHeader>
      <CardContent className="grid gap-4">
        <Field label="Mode" hint="MITM proxy needs CA cert + system proxy. Chromium launches a controlled browser — no setup.">
          <Select value={mode} onValueChange={(v) => setMode(v as CaptureMode)}>
            <SelectTrigger className="w-full">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="mitm">MITM proxy</SelectItem>
              <SelectItem value="chromium">Chromium browser (experimental)</SelectItem>
            </SelectContent>
          </Select>
        </Field>

        {mode === 'mitm' && (
          <>
            <Toggle
              label="Proxy enabled"
              value={draft.proxy.enabled}
              onChange={(v) => setDraft({ ...draft, proxy: { ...draft.proxy, enabled: v } })}
            />
            <Field label="Address">
              <Input
                value={draft.proxy.addr}
                onChange={(e) => setDraft({ ...draft, proxy: { ...draft.proxy, addr: e.target.value } })}
                placeholder="127.0.0.1:23410"
              />
            </Field>
            <Field label="CA directory" hint="Where root certificate / keys are written.">
              <Input
                value={draft.proxy.ca_dir}
                onChange={(e) => setDraft({ ...draft, proxy: { ...draft.proxy, ca_dir: e.target.value } })}
              />
            </Field>
          </>
        )}

        {mode === 'chromium' && (
          <>
            <Field label="Browser executable" hint="Leave blank to auto-detect.">
              <Input
                value={chromium.executable}
                onChange={(e) => setChromium({ executable: e.target.value })}
                placeholder="/usr/bin/google-chrome"
              />
            </Field>
            <div className="flex items-center justify-between gap-2">
              <span className="text-xs text-muted-foreground">
                {detecting
                  ? 'Detecting…'
                  : detected === null
                    ? 'Click Detect to scan for installed browsers.'
                    : detected.length === 0
                      ? 'No Chromium-family browser detected.'
                      : `Detected: ${detected.map((d) => d.path).join(', ')}`}
              </span>
              <Button variant="outline" size="sm" onClick={probe} disabled={detecting}>
                {detecting ? 'Detecting…' : 'Detect'}
              </Button>
            </div>
            <Field label="User data dir" hint="Leave blank to use the default Akagi profile under your config dir.">
              <Input
                value={chromium.user_data_dir}
                onChange={(e) => setChromium({ user_data_dir: e.target.value })}
                placeholder="(default)"
              />
            </Field>
            <Field label="Start URL">
              <Input
                value={chromium.start_url}
                onChange={(e) => setChromium({ start_url: e.target.value })}
                placeholder="https://game.maj-soul.com/1/"
              />
            </Field>
            <Toggle
              label="Force Chrome for Testing"
              value={chromium.force_cft}
              onChange={(v) => setChromium({ force_cft: v })}
            />
            <CftPanel chromium={chromium} setChromium={setChromium} />
          </>
        )}
      </CardContent>
    </Card>
  )
}

function CftPanel({
  chromium,
  setChromium,
}: {
  chromium: AppConfig['capture']['chromium']
  setChromium: (patch: Partial<AppConfig['capture']['chromium']>) => void
}) {
  const [installed, setInstalled] = useState<string[] | null>(null)
  const [busy, setBusy] = useState<'idle' | 'downloading' | 'removing'>('idle')

  const refresh = async () => {
    try {
      const list = await invoke<string[]>('list_cft_installed')
      setInstalled(list)
    } catch {
      setInstalled([])
    }
  }

  useEffect(() => {
    refresh()
  }, [])

  const download = async () => {
    setBusy('downloading')
    try {
      await invoke<string>('download_chrome_for_testing', {
        channel: chromium.cft_channel || 'stable',
      })
      await refresh()
    } catch (e) {
      console.error('CfT download failed:', e)
    } finally {
      setBusy('idle')
    }
  }

  const remove = async (version: string) => {
    setBusy('removing')
    try {
      await invoke('remove_chrome_for_testing', { version })
      await refresh()
    } catch (e) {
      console.error('CfT remove failed:', e)
    } finally {
      setBusy('idle')
    }
  }

  return (
    <div className="grid gap-2 rounded-md border border-border/50 p-3">
      <div className="flex items-center justify-between">
        <Label>Chrome for Testing</Label>
        <span className="text-xs text-muted-foreground">
          {installed === null
            ? 'Loading…'
            : installed.length === 0
              ? 'None installed'
              : `${installed.length} installed`}
        </span>
      </div>
      <Field label="Channel / version" hint='"stable" / "beta" / "dev" / "canary" or a literal version like "131.0.6778.85".'>
        <Input
          value={chromium.cft_channel}
          onChange={(e) => setChromium({ cft_channel: e.target.value })}
          placeholder="stable"
        />
      </Field>
      <div className="flex items-center justify-end gap-2">
        <Button variant="outline" size="sm" onClick={refresh} disabled={busy !== 'idle'}>
          Refresh
        </Button>
        <Button onClick={download} disabled={busy !== 'idle'} size="sm">
          {busy === 'downloading' ? 'Downloading…' : 'Download'}
        </Button>
      </div>
      {installed && installed.length > 0 && (
        <ul className="grid gap-1 text-sm">
          {installed.map((v) => (
            <li key={v} className="flex items-center justify-between rounded bg-muted/40 px-2 py-1">
              <span>{v}</span>
              <Button
                variant="ghost"
                size="sm"
                onClick={() => remove(v)}
                disabled={busy !== 'idle'}
              >
                Remove
              </Button>
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}
