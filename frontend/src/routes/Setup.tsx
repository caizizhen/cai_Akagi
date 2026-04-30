import { useEffect, useState } from 'react'
import { useNavigate, useSearchParams } from 'react-router-dom'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import { invoke } from '@/lib/tauri'
import { useConfigStore } from '@/stores/configStore'
import type { AppConfig, CaptureMode, DetectedBrowser } from '@/types'

type Step = 'welcome' | 'mode' | 'config' | 'finish'

const STEPS: Step[] = ['welcome', 'mode', 'config', 'finish']

export function Setup() {
  const stored = useConfigStore((s) => s.config)
  const setStored = useConfigStore((s) => s.setConfig)
  const [draft, setDraft] = useState<AppConfig | null>(stored)
  const [step, setStep] = useState<Step>('welcome')
  const [busy, setBusy] = useState(false)
  const [err, setErr] = useState<string | null>(null)
  const [params] = useSearchParams()
  const navigate = useNavigate()

  // True when the user is re-running setup from Settings (not first run).
  const isRerun = params.get('rerun') === '1' || stored?.general.first_run_completed === true

  useEffect(() => {
    if (stored) setDraft(stored)
  }, [stored])

  useEffect(() => {
    if (!stored) {
      invoke<AppConfig>('get_config').then(setStored).catch(() => {})
    }
  }, [stored, setStored])

  if (!draft) {
    return <div className="p-6 text-muted-foreground">Loading…</div>
  }

  const idx = STEPS.indexOf(step)
  const canBack = idx > 0
  const canNext = idx < STEPS.length - 1

  const next = () => setStep(STEPS[idx + 1])
  const back = () => setStep(STEPS[idx - 1])

  const finish = async () => {
    setBusy(true)
    setErr(null)
    try {
      const final: AppConfig = {
        ...draft,
        general: { ...draft.general, first_run_completed: true },
      }
      await invoke('update_config', { newConfig: final })
      setStored(final)
      navigate('/', { replace: true })
    } catch (e) {
      setErr(String(e))
    } finally {
      setBusy(false)
    }
  }

  const cancel = () => navigate('/', { replace: true })

  return (
    <div className="min-h-screen w-full flex items-center justify-center p-6">
      <Card className="w-full max-w-2xl">
        <CardHeader>
          <div className="flex items-center justify-between">
            <CardTitle>Akagi Setup</CardTitle>
            <span className="text-xs text-muted-foreground">Step {idx + 1} of {STEPS.length}</span>
          </div>
          <Stepper current={idx} />
        </CardHeader>
        <CardContent className="grid gap-6">
          {step === 'welcome' && <WelcomeStep />}
          {step === 'mode' && <ModeStep draft={draft} setDraft={setDraft} />}
          {step === 'config' && <ConfigStep draft={draft} setDraft={setDraft} />}
          {step === 'finish' && <FinishStep draft={draft} />}

          {err && (
            <div className="rounded-md border border-red-500/40 bg-red-500/10 px-3 py-2 text-sm text-red-400">
              {err}
            </div>
          )}

          <div className="flex justify-between">
            <div className="flex gap-2">
              {canBack ? (
                <Button variant="outline" onClick={back} disabled={busy}>Back</Button>
              ) : (
                <span />
              )}
              {isRerun && (
                <Button variant="ghost" onClick={cancel} disabled={busy}>Cancel</Button>
              )}
            </div>
            {canNext ? (
              <Button onClick={next} disabled={busy}>Next</Button>
            ) : (
              <Button onClick={finish} disabled={busy}>
                {busy ? 'Saving…' : 'Finish'}
              </Button>
            )}
          </div>
        </CardContent>
      </Card>
    </div>
  )
}

function Stepper({ current }: { current: number }) {
  return (
    <div className="flex gap-1.5 mt-3">
      {STEPS.map((_, i) => (
        <div
          key={i}
          className={`h-1 flex-1 rounded ${i <= current ? 'bg-primary' : 'bg-muted'}`}
        />
      ))}
    </div>
  )
}

function WelcomeStep() {
  return (
    <div className="grid gap-3">
      <h2 className="text-lg font-semibold">Welcome to Akagi</h2>
      <p className="text-sm text-muted-foreground">
        Akagi watches your mahjong game traffic and runs an mjai-protocol bot
        alongside it for realtime advice. Before the bot can do anything,
        Akagi needs a way to capture the game's WebSocket traffic.
      </p>
      <p className="text-sm text-muted-foreground">
        On the next page you'll pick between the legacy MITM proxy (powerful
        but requires CA-cert trust) and a controlled Chromium browser (no
        proxy / certificate setup — Akagi launches its own browser window).
      </p>
    </div>
  )
}

function ModeStep({
  draft,
  setDraft,
}: {
  draft: AppConfig
  setDraft: (c: AppConfig) => void
}) {
  const mode = draft.capture.mode
  return (
    <div className="grid gap-3">
      <h2 className="text-lg font-semibold">Pick a capture mode</h2>
      <ModeCard
        title="Chromium browser (recommended)"
        active={mode === 'chromium'}
        onClick={() => setDraft({ ...draft, capture: { ...draft.capture, mode: 'chromium' } })}
        description="Akagi launches a controlled browser and intercepts WebSocket frames via Chrome DevTools Protocol. No system proxy. No CA cert. Works for Mahjong Soul and Tenhou; not for RiichiCity (no web client)."
      />
      <ModeCard
        title="MITM proxy (advanced)"
        active={mode === 'mitm'}
        onClick={() => setDraft({ ...draft, capture: { ...draft.capture, mode: 'mitm' } })}
        description="hudsucker-based MITM. You configure your system proxy and trust Akagi's self-signed CA. Required for RiichiCity (no web client) and useful when you already have a browser session you don't want to disturb."
      />
    </div>
  )
}

function ModeCard({
  title,
  description,
  active,
  onClick,
}: {
  title: string
  description: string
  active: boolean
  onClick: () => void
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={`text-left rounded-md border p-4 transition-colors ${
        active ? 'border-primary bg-primary/5' : 'border-border hover:border-primary/40'
      }`}
    >
      <div className="flex items-center gap-2">
        <span
          className={`h-3 w-3 rounded-full border-2 ${
            active ? 'border-primary bg-primary' : 'border-muted-foreground/40'
          }`}
        />
        <span className="font-medium">{title}</span>
      </div>
      <p className="text-sm text-muted-foreground mt-2">{description}</p>
    </button>
  )
}

function ConfigStep({
  draft,
  setDraft,
}: {
  draft: AppConfig
  setDraft: (c: AppConfig) => void
}) {
  if (draft.capture.mode === 'mitm') {
    return (
      <div className="grid gap-3">
        <h2 className="text-lg font-semibold">MITM proxy settings</h2>
        <Field label="Listen address">
          <Input
            value={draft.proxy.addr}
            onChange={(e) => setDraft({ ...draft, proxy: { ...draft.proxy, addr: e.target.value } })}
            placeholder="127.0.0.1:23410"
          />
        </Field>
        <Field
          label="CA directory"
          hint={`The CA cert is generated on first start. Trust the .pem / .crt file in your OS or browser certificate store before pointing the game at this proxy.`}
        >
          <Input
            value={draft.proxy.ca_dir}
            onChange={(e) => setDraft({ ...draft, proxy: { ...draft.proxy, ca_dir: e.target.value } })}
          />
        </Field>
        <Field label="Proxy enabled" hint="Master switch — leave on for Akagi to start the proxy automatically.">
          <Select
            value={draft.proxy.enabled ? 'on' : 'off'}
            onValueChange={(v) => setDraft({ ...draft, proxy: { ...draft.proxy, enabled: v === 'on' } })}
          >
            <SelectTrigger className="w-full">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="on">On</SelectItem>
              <SelectItem value="off">Off</SelectItem>
            </SelectContent>
          </Select>
        </Field>
      </div>
    )
  }
  return <ChromiumConfigStep draft={draft} setDraft={setDraft} />
}

function ChromiumConfigStep({
  draft,
  setDraft,
}: {
  draft: AppConfig
  setDraft: (c: AppConfig) => void
}) {
  const chromium = draft.capture.chromium
  const setChromium = (patch: Partial<typeof chromium>) =>
    setDraft({
      ...draft,
      capture: { ...draft.capture, chromium: { ...chromium, ...patch } },
    })

  const [detected, setDetected] = useState<DetectedBrowser[] | null>(null)
  const [installed, setInstalled] = useState<string[] | null>(null)
  const [busy, setBusy] = useState<'idle' | 'detecting' | 'downloading'>('idle')

  const refresh = async () => {
    setBusy('detecting')
    try {
      const [d, i] = await Promise.all([
        invoke<DetectedBrowser[]>('detect_system_chrome'),
        invoke<string[]>('list_cft_installed'),
      ])
      setDetected(d)
      setInstalled(i)
    } catch {
      setDetected([])
      setInstalled([])
    } finally {
      setBusy('idle')
    }
  }

  useEffect(() => {
    refresh()
  }, [])

  const downloadCft = async () => {
    setBusy('downloading')
    try {
      await invoke('download_chrome_for_testing', { channel: chromium.cft_channel || 'stable' })
      await refresh()
    } catch {
      /* surfaced via notify */
    } finally {
      setBusy('idle')
    }
  }

  const ready = (detected && detected.length > 0) || (installed && installed.length > 0) || chromium.executable

  return (
    <div className="grid gap-3">
      <h2 className="text-lg font-semibold">Chromium settings</h2>
      <Field label="Browser executable" hint="Leave blank to auto-detect a system Chrome / Edge / Brave / Chromium.">
        <Input
          value={chromium.executable}
          onChange={(e) => setChromium({ executable: e.target.value })}
          placeholder="(auto-detect)"
        />
      </Field>
      <div className="rounded-md border border-border/50 p-3 grid gap-2">
        <div className="flex items-center justify-between">
          <Label>Detected browsers</Label>
          <Button variant="outline" size="sm" onClick={refresh} disabled={busy !== 'idle'}>
            {busy === 'detecting' ? 'Scanning…' : 'Refresh'}
          </Button>
        </div>
        {detected === null ? (
          <span className="text-xs text-muted-foreground">Scanning…</span>
        ) : detected.length === 0 ? (
          <span className="text-xs text-muted-foreground">None detected on this system.</span>
        ) : (
          <ul className="text-xs font-mono break-all">
            {detected.map((d) => (
              <li key={d.path}>· {d.path}</li>
            ))}
          </ul>
        )}
      </div>
      <div className="rounded-md border border-border/50 p-3 grid gap-2">
        <div className="flex items-center justify-between">
          <Label>Chrome for Testing</Label>
          <span className="text-xs text-muted-foreground">
            {installed === null ? 'Loading…' : installed.length === 0 ? 'None installed' : `${installed.length} installed`}
          </span>
        </div>
        <Field label="Channel / version" hint='"stable" / "beta" / "dev" / "canary" or a literal version like "131.0.6778.85".'>
          <Input
            value={chromium.cft_channel}
            onChange={(e) => setChromium({ cft_channel: e.target.value })}
            placeholder="stable"
          />
        </Field>
        <Button onClick={downloadCft} disabled={busy !== 'idle'} size="sm">
          {busy === 'downloading' ? 'Downloading…' : 'Download'}
        </Button>
      </div>
      <Field label="Start URL">
        <Input
          value={chromium.start_url}
          onChange={(e) => setChromium({ start_url: e.target.value })}
          placeholder="https://game.maj-soul.com/1/"
        />
      </Field>
      {!ready && (
        <div className="rounded-md border border-amber-500/40 bg-amber-500/10 px-3 py-2 text-sm text-amber-200">
          No browser is available yet. Either install Chrome / Edge / Brave / Chromium on your system, set an explicit executable above, or click Download to install Chrome for Testing.
        </div>
      )}
    </div>
  )
}

function FinishStep({ draft }: { draft: AppConfig }) {
  const m = draft.capture.mode
  return (
    <div className="grid gap-3">
      <h2 className="text-lg font-semibold">All set</h2>
      <div className="rounded-md border border-border/50 p-3 text-sm">
        <div><b>Mode:</b> {m === 'chromium' ? 'Chromium browser' : 'MITM proxy'}</div>
        {m === 'mitm' && (
          <>
            <div><b>Listen:</b> {draft.proxy.addr}</div>
            <div><b>CA dir:</b> {draft.proxy.ca_dir}</div>
          </>
        )}
        {m === 'chromium' && (
          <>
            <div><b>Executable:</b> {draft.capture.chromium.executable || '(auto-detect)'}</div>
            <div><b>Start URL:</b> {draft.capture.chromium.start_url}</div>
            <div><b>CfT channel:</b> {draft.capture.chromium.cft_channel}</div>
          </>
        )}
      </div>
      <p className="text-sm text-muted-foreground">
        Click <b>Finish</b> to save these settings. The capture backend will start automatically.
        You can change everything later in Settings → Capture, or rerun this wizard from there.
      </p>
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
