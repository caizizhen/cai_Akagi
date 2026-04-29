import { create } from 'zustand'
import type { LayoutItem } from 'react-grid-layout'
import {
  ALL_TILES,
  DEFAULT_HIDDEN,
  DEFAULT_HIDDEN_3P,
  DEFAULT_LAYOUTS,
  DEFAULT_LAYOUTS_3P,
  type Breakpoint,
  type TileId,
} from '@/tiles/defaults'

// Bump the version segment whenever DEFAULT_LAYOUTS changes shape so old
// saved data doesn't override the new defaults. v5 introduces per-mode
// (4p / 3p) layout slots; old v4 keys are orphaned in localStorage but
// harmless.
const LAYOUTS_KEY_4P = 'akagi.v5.layouts.4p'
const HIDDEN_KEY_4P = 'akagi.v5.hidden.4p'
const LAYOUTS_KEY_3P = 'akagi.v5.layouts.3p'
const HIDDEN_KEY_3P = 'akagi.v5.hidden.3p'

export type Layouts = Record<Breakpoint, LayoutItem[]>
export type HiddenSet = Record<Breakpoint, TileId[]>
export type GameMode = '4p' | '3p'

type LayoutStore = {
  /** Current display mode — selected by the dashboard from `game.num_players`. */
  mode: GameMode
  layouts4p: Layouts
  hidden4p: HiddenSet
  layouts3p: Layouts
  hidden3p: HiddenSet
  /** Active preset for the current `mode`. Computed; do not set directly. */
  layouts: Layouts
  hidden: HiddenSet
  setMode: (mode: GameMode) => void
  setLayouts: (l: Layouts) => void
  hide: (id: TileId, bp: Breakpoint) => void
  show: (id: TileId, bp: Breakpoint) => void
  reset: () => void
}

function deepClone<T>(v: T): T {
  return JSON.parse(JSON.stringify(v))
}

// Ensure all four breakpoints exist as arrays. Stale / partial localStorage
// data (older format, missing keys, non-array values) would otherwise crash
// downstream `.filter` calls in GameDashboard.
function normaliseLayouts(parsed: unknown, fallback: Layouts): Layouts {
  const fresh = deepClone(fallback)
  if (!parsed || typeof parsed !== 'object') return fresh
  const obj = parsed as Record<string, unknown>
  for (const bp of ['lg', 'md', 'sm', 'xs'] as const) {
    if (Array.isArray(obj[bp])) fresh[bp] = obj[bp] as Layouts[Breakpoint]
  }
  return fresh
}

function normaliseHidden(parsed: unknown, defaults: TileId[]): HiddenSet {
  const fallback: HiddenSet = {
    lg: [...defaults], md: [...defaults],
    sm: [...defaults], xs: [...defaults],
  }
  if (!parsed || typeof parsed !== 'object') return fallback
  const obj = parsed as Record<string, unknown>
  for (const bp of ['lg', 'md', 'sm', 'xs'] as const) {
    if (Array.isArray(obj[bp])) fallback[bp] = obj[bp] as TileId[]
  }
  return fallback
}

function loadLayouts(key: string, fallback: Layouts): Layouts {
  if (typeof localStorage === 'undefined') return deepClone(fallback)
  try {
    const raw = localStorage.getItem(key)
    if (!raw) return deepClone(fallback)
    return normaliseLayouts(JSON.parse(raw), fallback)
  } catch {
    return deepClone(fallback)
  }
}

function loadHidden(key: string, defaults: TileId[]): HiddenSet {
  if (typeof localStorage === 'undefined') return normaliseHidden(null, defaults)
  try {
    const raw = localStorage.getItem(key)
    if (!raw) return normaliseHidden(null, defaults)
    return normaliseHidden(JSON.parse(raw), defaults)
  } catch {
    return normaliseHidden(null, defaults)
  }
}

function persistMode(
  mode: GameMode,
  layouts: Layouts,
  hidden: HiddenSet,
) {
  if (typeof localStorage === 'undefined') return
  try {
    const lk = mode === '3p' ? LAYOUTS_KEY_3P : LAYOUTS_KEY_4P
    const hk = mode === '3p' ? HIDDEN_KEY_3P : HIDDEN_KEY_4P
    localStorage.setItem(lk, JSON.stringify(layouts))
    localStorage.setItem(hk, JSON.stringify(hidden))
  } catch {
    /* quota exceeded — ignore */
  }
}

function defaultEntry(id: TileId, bp: Breakpoint, mode: GameMode): LayoutItem {
  const presets = mode === '3p' ? DEFAULT_LAYOUTS_3P : DEFAULT_LAYOUTS
  const def = presets[bp].find((l) => l.i === id)
  if (def) return { ...def }
  return { i: id, x: 0, y: 999, w: 4, h: 4, minW: 2, minH: 2 }
}

export const useLayoutStore = create<LayoutStore>((set, get) => {
  const layouts4p = loadLayouts(LAYOUTS_KEY_4P, DEFAULT_LAYOUTS)
  const hidden4p = loadHidden(HIDDEN_KEY_4P, DEFAULT_HIDDEN)
  const layouts3p = loadLayouts(LAYOUTS_KEY_3P, DEFAULT_LAYOUTS_3P)
  const hidden3p = loadHidden(HIDDEN_KEY_3P, DEFAULT_HIDDEN_3P)
  return {
    mode: '4p',
    layouts4p,
    hidden4p,
    layouts3p,
    hidden3p,
    layouts: layouts4p,
    hidden: hidden4p,

    setMode: (mode) => {
      const s = get()
      if (s.mode === mode) return
      const layouts = mode === '3p' ? s.layouts3p : s.layouts4p
      const hidden = mode === '3p' ? s.hidden3p : s.hidden4p
      set({ mode, layouts, hidden })
    },

    setLayouts: (layouts) => {
      const s = get()
      persistMode(s.mode, layouts, s.hidden)
      set(
        s.mode === '3p'
          ? { layouts, layouts3p: layouts }
          : { layouts, layouts4p: layouts },
      )
    },

    hide: (id, bp) => {
      const s = get()
      const layouts = deepClone(s.layouts)
      layouts[bp] = (layouts[bp] ?? []).filter((l) => l.i !== id)
      const hidden = deepClone(s.hidden)
      hidden[bp] = [...new Set([...(hidden[bp] ?? []), id])]
      persistMode(s.mode, layouts, hidden)
      set(
        s.mode === '3p'
          ? { layouts, hidden, layouts3p: layouts, hidden3p: hidden }
          : { layouts, hidden, layouts4p: layouts, hidden4p: hidden },
      )
    },

    show: (id, bp) => {
      const s = get()
      const layouts = deepClone(s.layouts)
      if (!(layouts[bp] ?? []).some((l) => l.i === id)) {
        layouts[bp] = [...(layouts[bp] ?? []), defaultEntry(id, bp, s.mode)]
      }
      const hidden = deepClone(s.hidden)
      hidden[bp] = (hidden[bp] ?? []).filter((x) => x !== id)
      persistMode(s.mode, layouts, hidden)
      set(
        s.mode === '3p'
          ? { layouts, hidden, layouts3p: layouts, hidden3p: hidden }
          : { layouts, hidden, layouts4p: layouts, hidden4p: hidden },
      )
    },

    reset: () => {
      const s = get()
      const presets = s.mode === '3p' ? DEFAULT_LAYOUTS_3P : DEFAULT_LAYOUTS
      const defaultsHidden = s.mode === '3p' ? DEFAULT_HIDDEN_3P : DEFAULT_HIDDEN
      const layouts = deepClone(presets)
      const hidden: HiddenSet = {
        lg: [...defaultsHidden], md: [...defaultsHidden],
        sm: [...defaultsHidden], xs: [...defaultsHidden],
      }
      persistMode(s.mode, layouts, hidden)
      set(
        s.mode === '3p'
          ? { layouts, hidden, layouts3p: layouts, hidden3p: hidden }
          : { layouts, hidden, layouts4p: layouts, hidden4p: hidden },
      )
    },
  }
})

// Helper: compute visible tiles for a given breakpoint. `mode` controls
// which canonical tile order applies (3p drops `player-3` from the menu).
export function visibleTilesFor(
  bp: Breakpoint,
  hidden: HiddenSet,
  mode: GameMode = '4p',
): TileId[] {
  const hide = new Set(hidden[bp] ?? [])
  const tiles = mode === '3p' ? ALL_TILES.filter((id) => id !== 'player-3') : ALL_TILES
  return tiles.filter((id) => !hide.has(id))
}
