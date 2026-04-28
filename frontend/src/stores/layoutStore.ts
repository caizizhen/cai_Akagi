import { create } from 'zustand'
import type { LayoutItem } from 'react-grid-layout'
import {
  ALL_TILES,
  DEFAULT_HIDDEN,
  DEFAULT_LAYOUTS,
  type Breakpoint,
  type TileId,
} from '@/tiles/defaults'

const LAYOUTS_KEY = 'akagi.v2.layouts'
const HIDDEN_KEY = 'akagi.v2.hidden'

export type Layouts = Record<Breakpoint, LayoutItem[]>
export type HiddenSet = Record<Breakpoint, TileId[]>

type LayoutStore = {
  layouts: Layouts
  hidden: HiddenSet
  setLayouts: (l: Layouts) => void
  hide: (id: TileId, bp: Breakpoint) => void
  show: (id: TileId, bp: Breakpoint) => void
  reset: () => void
}

function deepClone<T>(v: T): T {
  return JSON.parse(JSON.stringify(v))
}

function loadLayouts(): Layouts {
  if (typeof localStorage === 'undefined') return deepClone(DEFAULT_LAYOUTS)
  try {
    const raw = localStorage.getItem(LAYOUTS_KEY)
    if (!raw) return deepClone(DEFAULT_LAYOUTS)
    const parsed = JSON.parse(raw)
    if (!parsed || typeof parsed !== 'object') return deepClone(DEFAULT_LAYOUTS)
    return parsed as Layouts
  } catch {
    return deepClone(DEFAULT_LAYOUTS)
  }
}

function loadHidden(): HiddenSet {
  const fallback: HiddenSet = { lg: [...DEFAULT_HIDDEN], md: [...DEFAULT_HIDDEN], sm: [...DEFAULT_HIDDEN], xs: [...DEFAULT_HIDDEN] }
  if (typeof localStorage === 'undefined') return fallback
  try {
    const raw = localStorage.getItem(HIDDEN_KEY)
    if (!raw) return fallback
    const parsed = JSON.parse(raw)
    if (!parsed || typeof parsed !== 'object') return fallback
    return parsed as HiddenSet
  } catch {
    return fallback
  }
}

function persist(layouts: Layouts, hidden: HiddenSet) {
  if (typeof localStorage === 'undefined') return
  try {
    localStorage.setItem(LAYOUTS_KEY, JSON.stringify(layouts))
    localStorage.setItem(HIDDEN_KEY, JSON.stringify(hidden))
  } catch {
    /* quota exceeded — ignore */
  }
}

function defaultEntry(id: TileId, bp: Breakpoint): LayoutItem {
  const def = DEFAULT_LAYOUTS[bp].find((l) => l.i === id)
  if (def) return { ...def }
  // fallback for an id not defined for this bp
  return { i: id, x: 0, y: 999, w: 4, h: 4, minW: 2, minH: 2 }
}

export const useLayoutStore = create<LayoutStore>((set, get) => ({
  layouts: loadLayouts(),
  hidden: loadHidden(),

  setLayouts: (layouts) => {
    persist(layouts, get().hidden)
    set({ layouts })
  },

  hide: (id, bp) => {
    const layouts = deepClone(get().layouts)
    layouts[bp] = (layouts[bp] ?? []).filter((l) => l.i !== id)
    const hidden = deepClone(get().hidden)
    hidden[bp] = [...new Set([...(hidden[bp] ?? []), id])]
    persist(layouts, hidden)
    set({ layouts, hidden })
  },

  show: (id, bp) => {
    const layouts = deepClone(get().layouts)
    if (!(layouts[bp] ?? []).some((l) => l.i === id)) {
      layouts[bp] = [...(layouts[bp] ?? []), defaultEntry(id, bp)]
    }
    const hidden = deepClone(get().hidden)
    hidden[bp] = (hidden[bp] ?? []).filter((x) => x !== id)
    persist(layouts, hidden)
    set({ layouts, hidden })
  },

  reset: () => {
    const layouts = deepClone(DEFAULT_LAYOUTS)
    const hidden: HiddenSet = {
      lg: [...DEFAULT_HIDDEN], md: [...DEFAULT_HIDDEN],
      sm: [...DEFAULT_HIDDEN], xs: [...DEFAULT_HIDDEN],
    }
    persist(layouts, hidden)
    set({ layouts, hidden })
  },
}))

// Helper: compute visible tiles for a given breakpoint.
export function visibleTilesFor(bp: Breakpoint, hidden: HiddenSet): TileId[] {
  const hide = new Set(hidden[bp] ?? [])
  return ALL_TILES.filter((id) => !hide.has(id))
}
