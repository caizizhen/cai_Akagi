// Platform metadata used by the Setup wizard, Settings page, and any
// UI that needs to show platform-specific defaults / hints.
//
// `kind` mirrors `src/config/platform.rs::Platform` (PascalCase JSON), so
// the values flow into `AppConfig.platform.kind` unchanged.

import type { PlatformKind } from '@/types'

export type PlatformInfo = {
  kind: PlatformKind
  /** Display name shown in pickers. */
  label: string
  /** One-line summary shown beside the picker. */
  description: string
  /**
   * Default URL for the Chromium capture backend's `start_url` when this
   * platform is active. Picked so the launched browser lands directly on
   * the game's lobby/match-find page.
   */
  defaultStartUrl: string
}

export const PLATFORMS: PlatformInfo[] = [
  {
    kind: 'Majsoul',
    label: 'Mahjong Soul',
    description: 'game.maj-soul.com — yonma + sanma, ranked + friend rooms.',
    defaultStartUrl: 'https://game.maj-soul.com/1/',
  },
  {
    kind: 'Tenhou',
    label: 'Tenhou',
    description: 'tenhou.net — yonma + sanma',
    defaultStartUrl: 'https://tenhou.net/4/',
  },
]

const BY_KIND: Record<PlatformKind, PlatformInfo> = Object.fromEntries(
  PLATFORMS.map((p) => [p.kind, p]),
) as Record<PlatformKind, PlatformInfo>

export function platformInfo(kind: PlatformKind): PlatformInfo {
  return BY_KIND[kind]
}

/**
 * Set of every URL we have ever shipped as a "default" for a platform.
 * Used to decide whether `start_url` is still a known default (so it's
 * safe to swap for the new platform's default on a platform change) or
 * whether the user has customised it (in which case we leave it alone).
 */
const KNOWN_DEFAULT_URLS = new Set<string>(PLATFORMS.map((p) => p.defaultStartUrl))

export function isKnownDefaultStartUrl(url: string): boolean {
  return KNOWN_DEFAULT_URLS.has(url.trim())
}
