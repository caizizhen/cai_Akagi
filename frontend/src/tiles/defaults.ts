import type { LayoutItem } from 'react-grid-layout'

export type TileId =
  | 'header'
  | 'player-0'
  | 'player-1'
  | 'player-2'
  | 'player-3'
  | 'self-hand'
  | 'recommendations'
  | 'risk-chart'
  | 'opponents'
  | 'events'
  | 'notifications'
  | 'bot-responses'
  | 'proxy-control'

export type Breakpoint = 'lg' | 'md' | 'sm' | 'xs'

export const BREAKPOINTS: Record<Breakpoint, number> = { lg: 1200, md: 996, sm: 768, xs: 0 }
export const COLS: Record<Breakpoint, number> = { lg: 12, md: 10, sm: 6, xs: 4 }

// All tile ids in canonical order.
export const ALL_TILES: TileId[] = [
  'header',
  'player-0',
  'player-1',
  'player-2',
  'player-3',
  'self-hand',
  'recommendations',
  'risk-chart',
  'opponents',
  'events',
  'notifications',
  'bot-responses',
  'proxy-control',
]

export const DEFAULT_HIDDEN: TileId[] = []

// Default layout for the lg (12-col) breakpoint. RGL Responsive copies
// layouts from lg → md → sm → xs when missing entries; for now we provide
// only lg + xs and let the responsive layout handle md / sm dynamically.
const LG_LAYOUT: LayoutItem[] = [
  { i: 'header',          x: 0, y: 0,  w: 12, h: 3, minW: 6, minH: 2, maxH: 5 },
  { i: 'player-0',        x: 0, y: 2,  w: 3,  h: 8, minW: 2, minH: 6 },
  { i: 'player-1',        x: 3, y: 2,  w: 3,  h: 8, minW: 2, minH: 6 },
  { i: 'player-3',        x: 6, y: 2,  w: 3,  h: 8, minW: 2, minH: 6 },
  { i: 'recommendations', x: 9, y: 2,  w: 3,  h: 8, minW: 2, minH: 4 },
  { i: 'player-2',        x: 0, y: 10, w: 6,  h: 6, minW: 3, minH: 4 },
  { i: 'risk-chart',      x: 6, y: 10, w: 3,  h: 6, minW: 2, minH: 4 },
  { i: 'opponents',       x: 9, y: 10, w: 3,  h: 6, minW: 2, minH: 4 },
  { i: 'self-hand',       x: 0, y: 16, w: 12, h: 4, minW: 6, minH: 2, maxH: 6 },
  { i: 'events',          x: 0, y: 19, w: 3,  h: 5, minW: 2, minH: 3 },
  { i: 'notifications',   x: 3, y: 19, w: 3,  h: 5, minW: 2, minH: 3 },
  { i: 'proxy-control',   x: 6, y: 19, w: 3,  h: 5, minW: 2, minH: 2, maxH: 6 },
  { i: 'bot-responses',   x: 9, y: 19, w: 3,  h: 5, minW: 2, minH: 3 },
]

const MD_LAYOUT: LayoutItem[] = [
  { i: 'header',          x: 0, y: 0,  w: 10, h: 3, minW: 5, minH: 2, maxH: 5 },
  { i: 'player-0',        x: 0, y: 2,  w: 3,  h: 8, minW: 2, minH: 6 },
  { i: 'player-1',        x: 3, y: 2,  w: 3,  h: 8, minW: 2, minH: 6 },
  { i: 'player-3',        x: 6, y: 2,  w: 4,  h: 8, minW: 2, minH: 6 },
  { i: 'recommendations', x: 0, y: 10, w: 5,  h: 6, minW: 2, minH: 4 },
  { i: 'player-2',        x: 5, y: 10, w: 5,  h: 6, minW: 3, minH: 4 },
  { i: 'risk-chart',      x: 0, y: 16, w: 5,  h: 6, minW: 2, minH: 4 },
  { i: 'opponents',       x: 5, y: 16, w: 5,  h: 6, minW: 2, minH: 4 },
  { i: 'self-hand',       x: 0, y: 22, w: 10, h: 4, minW: 5, minH: 2, maxH: 6 },
  { i: 'events',          x: 0, y: 25, w: 5,  h: 5, minW: 2, minH: 3 },
  { i: 'notifications',   x: 5, y: 25, w: 5,  h: 5, minW: 2, minH: 3 },
  { i: 'proxy-control',   x: 0, y: 30, w: 5,  h: 5, minW: 2, minH: 2, maxH: 6 },
  { i: 'bot-responses',   x: 5, y: 30, w: 5,  h: 5, minW: 2, minH: 3 },
]

const SM_LAYOUT: LayoutItem[] = [
  { i: 'header',          x: 0, y: 0,  w: 6, h: 3, minW: 4, minH: 2, maxH: 5 },
  { i: 'player-0',        x: 0, y: 2,  w: 3, h: 8, minW: 2, minH: 6 },
  { i: 'player-1',        x: 3, y: 2,  w: 3, h: 8, minW: 2, minH: 6 },
  { i: 'player-3',        x: 0, y: 10, w: 3, h: 8, minW: 2, minH: 6 },
  { i: 'recommendations', x: 3, y: 10, w: 3, h: 8, minW: 2, minH: 4 },
  { i: 'player-2',        x: 0, y: 18, w: 6, h: 6, minW: 3, minH: 4 },
  { i: 'risk-chart',      x: 0, y: 24, w: 3, h: 6, minW: 2, minH: 4 },
  { i: 'opponents',       x: 3, y: 24, w: 3, h: 6, minW: 2, minH: 4 },
  { i: 'self-hand',       x: 0, y: 30, w: 6, h: 4, minW: 4, minH: 2, maxH: 6 },
  { i: 'events',          x: 0, y: 33, w: 3, h: 5, minW: 2, minH: 3 },
  { i: 'notifications',   x: 3, y: 33, w: 3, h: 5, minW: 2, minH: 3 },
  { i: 'proxy-control',   x: 0, y: 38, w: 3, h: 5, minW: 2, minH: 2, maxH: 6 },
  { i: 'bot-responses',   x: 3, y: 38, w: 3, h: 5, minW: 2, minH: 3 },
]

const XS_LAYOUT: LayoutItem[] = ALL_TILES.map((id, i) => ({
  i: id,
  x: 0,
  y: i * 6,
  w: 4,
  h: id === 'header' || id === 'self-hand' ? 4 : 6,
  minW: 4,
  minH: 2,
}))

export const DEFAULT_LAYOUTS: Record<Breakpoint, LayoutItem[]> = {
  lg: LG_LAYOUT,
  md: MD_LAYOUT,
  sm: SM_LAYOUT,
  xs: XS_LAYOUT,
}

export const TILE_TITLES: Record<TileId, string> = {
  'header':          'Game Header',
  'player-0':        'Player 1',
  'player-1':        'Player 2',
  'player-2':        'Player 3 (Self)',
  'player-3':        'Player 4',
  'self-hand':       'Self Hand',
  'recommendations': 'Recommendations',
  'risk-chart':      'Mixed Risk',
  'opponents':       'Opponents',
  'events':          'Game Events',
  'notifications':   'Notifications',
  'bot-responses':   'Bot Responses',
  'proxy-control':   'Proxy',
}
