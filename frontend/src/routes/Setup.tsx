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
import { Toaster } from '@/components/ui/sonner'
import { invoke } from '@/lib/tauri'
import { useTauriBridge } from '@/hooks/useTauriBridge'
import { useConfigStore } from '@/stores/configStore'
import { ManifestField } from '@/components/ManifestField'
import { PLATFORMS, platformInfo } from '@/lib/platforms'
import type { AppConfig, BotInfo, BotSettings, DetectedBrowser, PlatformKind } from '@/types'

type Step = 'welcome' | 'platform' | 'mode' | 'config' | 'bots' | 'configure' | 'finish'

const STEPS: Step[] = ['welcome', 'platform', 'mode', 'config', 'bots', 'configure', 'finish']

// Author-provided MJAI bots installed by the first-run wizard. Same
// install path as the manual Bots → Install From GitHub flow, just
// pre-filled with author defaults.
const BOT_REPO = 'shinkuan/Akagi-MjaiBot-Mortal'
const BOT_4P_NAME = 'mortal'
const BOT_3P_NAME = 'mortal3p'
const BOT_4P_ASSET = 'release4p.zip'
const BOT_3P_ASSET = 'release3p.zip'

export function Setup() {
  // The wizard renders standalone (no <App> parent), so we wire the
  // tauri event bridge + toast surface here ourselves. Without this the
  // CfT download progress notifications wouldn't show up during setup.
  useTauriBridge()
  const stored = useConfigStore((s) => s.config)
  const setStored = useConfigStore((s) => s.setConfig)
  const [draft, setDraft] = useState<AppConfig | null>(stored)
  const [step, setStep] = useState<Step>('welcome')
  const [busy, setBusy] = useState(false)
  const [err, setErr] = useState<string | null>(null)
  const [params] = useSearchParams()
  const navigate = useNavigate()
  // Bot settings drafts keyed by bot name. Populated lazily when the
  // configure step mounts (ConfigureBotsStep loads via get_bot_settings).
  // Lifted here so `next()` can flush them to disk before advancing.
  // MUST live above the early-return below — React forbids skipping a
  // hook on first render and then calling it on subsequent renders.
  const [botSettingsDraft, setBotSettingsDraft] = useState<Record<string, BotSettings>>({})

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

  const saveBotSettings = async () => {
    for (const [name, settings] of Object.entries(botSettingsDraft)) {
      try {
        await invoke('update_bot_settings', { name, values: settings.values })
      } catch (e) {
        // Surface the first failure to the wizard's error strip; let
        // the user retry. Don't proceed to Finish with stale settings.
        throw new Error(`Save failed for ${name}: ${e}`)
      }
    }
  }

  const next = async () => {
    if (step === 'configure') {
      setBusy(true)
      setErr(null)
      try {
        await saveBotSettings()
      } catch (e) {
        setErr(String(e))
        setBusy(false)
        return
      }
      setBusy(false)
    }
    setStep(STEPS[idx + 1])
  }
  const back = () => setStep(STEPS[idx - 1])

  const finish = async () => {
    setBusy(true)
    setErr(null)
    try {
      // Re-query the bot list so the chosen active_4p / active_3p
      // reflect what's *actually* installed right now, regardless of
      // whether the user installed in this wizard run or already had
      // bots from a previous install.
      let installed: BotInfo[] = []
      try {
        installed = await invoke<BotInfo[]>('list_bots')
      } catch {
        /* ignore: bot dir may not be set up — we'll just leave the
           active_* fields at whatever the user had before. */
      }
      const has4p = installed.some((b) => b.name === BOT_4P_NAME)
      const has3p = installed.some((b) => b.name === BOT_3P_NAME)

      const final: AppConfig = {
        ...draft,
        general: { ...draft.general, first_run_completed: true },
        bot: {
          ...draft.bot,
          // Auto-enable + select author bots when the wizard installed
          // them. Don't downgrade an existing custom config (e.g. user
          // re-runs setup but keeps their own bot.active_4p): only
          // touch active_* when the corresponding bot is present.
          enabled: draft.bot.enabled || has4p || has3p,
          active_4p: has4p ? BOT_4P_NAME : draft.bot.active_4p,
          active_3p: has3p ? BOT_3P_NAME : draft.bot.active_3p,
        },
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
      <Toaster />
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
          {step === 'platform' && <PlatformStep draft={draft} setDraft={setDraft} />}
          {step === 'mode' && <ModeStep draft={draft} setDraft={setDraft} />}
          {step === 'config' && <ConfigStep draft={draft} setDraft={setDraft} />}
          {step === 'bots' && <BotsStep />}
          {step === 'configure' && (
            <ConfigureBotsStep
              drafts={botSettingsDraft}
              setDrafts={setBotSettingsDraft}
            />
          )}
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

function PlatformStep({
  draft,
  setDraft,
}: {
  draft: AppConfig
  setDraft: (c: AppConfig) => void
}) {
  const current = draft.platform.kind
  // Switching platforms inside the first-run wizard always rewrites
  // chromium.start_url to the new platform's default — a user that's
  // walking through the wizard hasn't had a chance to customise yet,
  // and an old default left over from a previous platform pick is
  // strictly wrong (it would land the launched browser on the wrong
  // game). Re-customisation, if needed, happens on the Chromium config
  // step that comes next.
  const pick = (kind: PlatformKind) => {
    if (kind === current) return
    setDraft({
      ...draft,
      platform: { kind },
      capture: {
        ...draft.capture,
        chromium: {
          ...draft.capture.chromium,
          start_url: platformInfo(kind).defaultStartUrl,
        },
      },
    })
  }

  return (
    <div className="grid gap-3">
      <h2 className="text-lg font-semibold">Pick your platform</h2>
      <p className="text-sm text-muted-foreground">
        Akagi listens for one game's WebSocket protocol at a time. You can change this later in Settings.
      </p>
      {PLATFORMS.map((p) => (
        <ModeCard
          key={p.kind}
          title={p.label}
          active={current === p.kind}
          onClick={() => pick(p.kind)}
          description={p.description}
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
      await invoke<string>('download_chrome_for_testing', {
        channel: chromium.cft_channel || 'stable',
      })
      // Explicit download in the wizard = explicit opt-in to CfT.
      // Without this, a user who has both system Chrome and a freshly
      // downloaded CfT would still launch the system Chrome (system
      // takes priority unless force_cft is on), which is exactly what
      // the "Download" button was meant to override.
      setChromium({ force_cft: true })
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
      <Field
        label="Start URL"
        hint={`Default for ${platformInfo(draft.platform.kind).label}: ${platformInfo(draft.platform.kind).defaultStartUrl}`}
      >
        <Input
          value={chromium.start_url}
          onChange={(e) => setChromium({ start_url: e.target.value })}
          placeholder={platformInfo(draft.platform.kind).defaultStartUrl}
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

function BotsStep() {
  const [installed, setInstalled] = useState<BotInfo[] | null>(null)
  const [installing, setInstalling] = useState<'4p' | '3p' | null>(null)
  const [errors, setErrors] = useState<{ [k: string]: string }>({})

  const refresh = async () => {
    try {
      const list = await invoke<BotInfo[]>('list_bots')
      setInstalled(list)
    } catch {
      setInstalled([])
    }
  }

  useEffect(() => {
    refresh()
  }, [])

  const has4p = installed?.some((b) => b.name === BOT_4P_NAME) ?? false
  const has3p = installed?.some((b) => b.name === BOT_3P_NAME) ?? false

  const install = async (mode: '4p' | '3p') => {
    setInstalling(mode)
    setErrors((e) => {
      const { [mode]: _, ...rest } = e
      return rest
    })
    try {
      await invoke('install_bot_from_github', {
        repo: BOT_REPO,
        assetGlob: mode === '4p' ? BOT_4P_ASSET : BOT_3P_ASSET,
        name: mode === '4p' ? BOT_4P_NAME : BOT_3P_NAME,
      })
      await refresh()
    } catch (e) {
      setErrors((prev) => ({ ...prev, [mode]: String(e) }))
    } finally {
      setInstalling(null)
    }
  }

  return (
    <div className="grid gap-3">
      <h2 className="text-lg font-semibold">Install MJAI bots</h2>
      <p className="text-sm text-muted-foreground">
        An MJAI bot is required for game analysis — without one, Akagi only
        captures traffic. The author publishes pre-built Mortal-based bots
        for both 4-player and 3-player modes; install one or both here.
        Skip this step if you'd rather build your own.
      </p>
      <div className="rounded-md border border-amber-500/40 bg-amber-500/10 px-3 py-2 text-xs text-amber-200">
        <b>License notice:</b> Akagi-MjaiBot-Mortal is distributed under{' '}
        <span className="font-mono">AGPL-3.0</span>. By installing, you agree
        to its terms — most importantly, distributing modified versions
        requires releasing your modifications under AGPL-3.0 as well.
      </div>
      <BotInstallCard
        title="4-player (yonma) — mortal"
        description="Mortal bot tuned for 4-player tables."
        installed={has4p}
        installing={installing === '4p'}
        disabled={installing !== null}
        error={errors['4p']}
        onInstall={() => install('4p')}
      />
      <BotInstallCard
        title="3-player (sanma) — mortal3p"
        description="Mortal RL bot tuned for 3-player tables."
        installed={has3p}
        installing={installing === '3p'}
        disabled={installing !== null}
        error={errors['3p']}
        onInstall={() => install('3p')}
      />
      <p className="text-xs text-muted-foreground">
        Installation runs <span className="font-mono">uv sync</span> on the
        bot's <span className="font-mono">pyproject.toml</span> and may take
        a minute on first run. Progress shows up as a toast in the corner.
      </p>
    </div>
  )
}

function BotInstallCard({
  title,
  description,
  installed,
  installing,
  disabled,
  error,
  onInstall,
}: {
  title: string
  description: string
  installed: boolean
  installing: boolean
  disabled: boolean
  error?: string
  onInstall: () => void
}) {
  return (
    <div className="rounded-md border p-3 grid gap-2">
      <div className="flex items-center justify-between gap-2">
        <div>
          <div className="font-medium">{title}</div>
          <div className="text-xs text-muted-foreground">{description}</div>
        </div>
        {installed ? (
          <span className="text-xs px-2 py-1 rounded-full bg-emerald-500/15 text-emerald-300 border border-emerald-500/30">
            Installed
          </span>
        ) : (
          <Button onClick={onInstall} disabled={disabled} size="sm">
            {installing ? 'Installing…' : 'Install'}
          </Button>
        )}
      </div>
      {error && (
        <div className="text-xs text-red-400 font-mono break-all">{error}</div>
      )}
    </div>
  )
}

function ConfigureBotsStep({
  drafts,
  setDrafts,
}: {
  drafts: Record<string, BotSettings>
  setDrafts: React.Dispatch<React.SetStateAction<Record<string, BotSettings>>>
}) {
  const [installed, setInstalled] = useState<BotInfo[] | null>(null)
  const [loadErrors, setLoadErrors] = useState<Record<string, string>>({})

  // Pull the bot list, then for each bot with a manifest fetch its
  // current values into the wizard draft. Skip bots that already have a
  // draft so back-and-forth navigation doesn't clobber unsaved edits.
  useEffect(() => {
    let cancelled = false
    ;(async () => {
      let list: BotInfo[]
      try {
        list = await invoke<BotInfo[]>('list_bots')
      } catch {
        if (!cancelled) setInstalled([])
        return
      }
      if (cancelled) return
      setInstalled(list)
      const targets = list.filter(
        (b) => (b.name === BOT_4P_NAME || b.name === BOT_3P_NAME) && b.manifest,
      )
      for (const b of targets) {
        if (drafts[b.name]) continue
        try {
          const s = await invoke<BotSettings>('get_bot_settings', { name: b.name })
          if (cancelled) return
          setDrafts((prev) => ({ ...prev, [b.name]: s }))
        } catch (e) {
          if (cancelled) return
          setLoadErrors((prev) => ({ ...prev, [b.name]: String(e) }))
        }
      }
    })()
    return () => {
      cancelled = true
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  const wizardBots = (installed ?? []).filter(
    (b) => b.name === BOT_4P_NAME || b.name === BOT_3P_NAME,
  )

  if (installed === null) {
    return <div className="text-sm text-muted-foreground">Loading bot settings…</div>
  }

  if (wizardBots.length === 0) {
    return (
      <div className="grid gap-3">
        <h2 className="text-lg font-semibold">Configure bots</h2>
        <p className="text-sm text-muted-foreground">
          No author-provided bots are installed. Skip this step or go back to
          install one. You can always tune bot settings later under Bots →
          Configure.
        </p>
      </div>
    )
  }

  return (
    <div className="grid gap-4">
      <h2 className="text-lg font-semibold">Configure bots</h2>
      <p className="text-sm text-muted-foreground">
        Defaults are usually fine — these knobs let you swap models, tune
        latency, and toggle bot behaviours. You can change them later under
        Bots → Configure.
      </p>
      {wizardBots.map((b) => (
        <BotSettingsForm
          key={b.name}
          name={b.name}
          loadError={loadErrors[b.name]}
          settings={drafts[b.name]}
          onChange={(values) =>
            setDrafts((prev) => {
              const cur = prev[b.name]
              if (!cur) return prev
              return { ...prev, [b.name]: { ...cur, values } }
            })
          }
        />
      ))}
    </div>
  )
}

function BotSettingsForm({
  name,
  settings,
  loadError,
  onChange,
}: {
  name: string
  settings: BotSettings | undefined
  loadError?: string
  onChange: (values: Record<string, unknown>) => void
}) {
  const title = settings?.manifest.bot.display ?? name
  const description = settings?.manifest.bot.description

  if (loadError) {
    return (
      <div className="rounded-md border border-red-500/40 bg-red-500/10 p-3">
        <div className="font-medium">{title}</div>
        <div className="text-xs text-red-400 font-mono break-all mt-1">{loadError}</div>
      </div>
    )
  }
  if (!settings) {
    return (
      <div className="rounded-md border p-3 text-sm text-muted-foreground">
        Loading {name}…
      </div>
    )
  }

  const entries = Object.entries(settings.manifest.settings)
  return (
    <div className="rounded-md border p-3 grid gap-3">
      <div>
        <div className="font-medium">{title}</div>
        {description && <div className="text-xs text-muted-foreground">{description}</div>}
      </div>
      {entries.length === 0 ? (
        <div className="text-xs text-muted-foreground">No tunable settings.</div>
      ) : (
        <div className="grid gap-3">
          {entries.map(([key, spec]) => (
            <ManifestField
              key={key}
              fieldKey={key}
              spec={spec}
              value={settings.values[key] ?? spec.default}
              onChange={(v) => onChange({ ...settings.values, [key]: v })}
            />
          ))}
        </div>
      )}
    </div>
  )
}

function FinishStep({ draft }: { draft: AppConfig }) {
  const m = draft.capture.mode
  const [installed, setInstalled] = useState<BotInfo[] | null>(null)
  useEffect(() => {
    invoke<BotInfo[]>('list_bots').then(setInstalled).catch(() => setInstalled([]))
  }, [])
  const has4p = installed?.some((b) => b.name === BOT_4P_NAME) ?? false
  const has3p = installed?.some((b) => b.name === BOT_3P_NAME) ?? false
  const botSummary = has4p && has3p
    ? `${BOT_4P_NAME} (4P), ${BOT_3P_NAME} (3P)`
    : has4p
      ? `${BOT_4P_NAME} (4P)`
      : has3p
        ? `${BOT_3P_NAME} (3P)`
        : 'none — analysis will be unavailable'
  return (
    <div className="grid gap-3">
      <h2 className="text-lg font-semibold">All set</h2>
      <div className="rounded-md border border-border/50 p-3 text-sm">
        <div><b>Platform:</b> {platformInfo(draft.platform.kind).label}</div>
        <div><b>Mode:</b> {m === 'chromium' ? 'Chromium browser' : 'MITM proxy'}</div>
        {m === 'mitm' && (
          <>
            <div><b>Listen:</b> {draft.proxy.addr}</div>
            <div><b>CA dir:</b> {draft.proxy.ca_dir}</div>
          </>
        )}
        {m === 'chromium' && (
          <>
            <div>
              <b>Executable:</b>{' '}
              {draft.capture.chromium.executable
                ? draft.capture.chromium.executable
                : draft.capture.chromium.force_cft
                  ? `(Chrome for Testing — ${draft.capture.chromium.cft_channel || 'stable'})`
                  : '(auto-detect: system → Chrome for Testing)'}
            </div>
            <div><b>Start URL:</b> {draft.capture.chromium.start_url}</div>
            <div><b>CfT channel:</b> {draft.capture.chromium.cft_channel}</div>
          </>
        )}
        <div><b>Bots:</b> {installed === null ? 'checking…' : botSummary}</div>
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
