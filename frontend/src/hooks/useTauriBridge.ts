import { useEffect } from 'react'
import { HAS_TAURI, invoke, listen } from '@/lib/tauri'
import type {
  AnalysisResult,
  BotResponse,
  BotStatus,
  GameStateSnapshot,
  MahgenView,
  MjaiEvent,
  Notification,
  ProxyStatus,
  Snapshot,
} from '@/types'
import { useGameStore } from '@/stores/gameStore'
import { useAnalysisStore } from '@/stores/analysisStore'
import { useBotStore } from '@/stores/botStore'
import { useProxyStore } from '@/stores/proxyStore'
import { useNotifyStore } from '@/stores/notifyStore'
import { useConfigStore } from '@/stores/configStore'

// One-shot bridge mounted from <App>. Subscribes to all Tauri events,
// hydrates initial state, and unsubscribes on unmount.
export function useTauriBridge() {
  useEffect(() => {
    if (!HAS_TAURI) return

    const unlistens: Array<() => void> = []
    let cancelled = false

    const refreshGame = async () => {
      try {
        const [snap, view] = await Promise.all([
          invoke<GameStateSnapshot | null>('get_game_snapshot'),
          invoke<MahgenView | null>('get_mahgen_view'),
        ])
        if (cancelled) return
        useGameStore.getState().setGame(snap)
        useGameStore.getState().setView(view)
      } catch {
        /* ignore: backend may not be ready */
      }
    }

    ;(async () => {
      try {
        const status = await invoke<Snapshot>('get_status')
        if (cancelled) return
        useConfigStore.getState().setConfig(status.config)
        useConfigStore.getState().setLogDir(status.log_dir)
        useBotStore.getState().setStatus(status.bot_status)
        useProxyStore.getState().set(status.proxy_status)
      } catch {
        /* ignore */
      }
      await refreshGame()
      try {
        const a = await invoke<AnalysisResult | null>('get_analysis')
        if (!cancelled) useAnalysisStore.getState().set(a)
      } catch {
        /* ignore */
      }
    })()

    listen<MjaiEvent>('mjai-event', (e) => {
      useNotifyStore.getState().pushEvent(e)
      void refreshGame()
    }).then((u) => unlistens.push(u))

    listen<AnalysisResult>('analysis-result', (a) => {
      useAnalysisStore.getState().set(a)
    }).then((u) => unlistens.push(u))

    listen<BotStatus>('bot-status', (s) => {
      useBotStore.getState().setStatus(s)
    }).then((u) => unlistens.push(u))

    listen<ProxyStatus>('proxy-status', (s) => {
      useProxyStore.getState().set(s)
    }).then((u) => unlistens.push(u))

    listen<BotResponse>('bot-response', (r) => {
      useNotifyStore.getState().pushResponse(r)
    }).then((u) => unlistens.push(u))

    listen<Notification>('notify', (n) => {
      useNotifyStore.getState().pushToast(n)
    }).then((u) => unlistens.push(u))

    return () => {
      cancelled = true
      unlistens.forEach((u) => u())
    }
  }, [])
}
