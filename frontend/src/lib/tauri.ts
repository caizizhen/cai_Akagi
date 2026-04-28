import { invoke as tauriInvoke } from '@tauri-apps/api/core'
import { listen as tauriListen, type UnlistenFn } from '@tauri-apps/api/event'

const TAURI_GLOBAL = (globalThis as unknown as { __TAURI__?: unknown }).__TAURI__
export const HAS_TAURI = TAURI_GLOBAL !== undefined

export async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (!HAS_TAURI) throw new Error(`tauri not available: ${cmd}`)
  return await tauriInvoke<T>(cmd, args)
}

export async function listen<T>(
  name: string,
  cb: (payload: T) => void,
): Promise<UnlistenFn> {
  if (!HAS_TAURI) return () => {}
  return await tauriListen<T>(name, (e) => cb(e.payload))
}
