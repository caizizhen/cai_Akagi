import { useState } from 'react'
import {
  Responsive,
  useContainerWidth,
  verticalCompactor,
  type Layout,
  type LayoutItem,
  type ResponsiveLayouts,
} from 'react-grid-layout'
import 'react-grid-layout/css/styles.css'
import 'react-resizable/css/styles.css'

import { Button } from '@/components/ui/button'
import { useLayoutStore, visibleTilesFor } from '@/stores/layoutStore'
import {
  BREAKPOINTS,
  COLS,
  type Breakpoint,
  type TileId,
} from '@/tiles/defaults'
import { renderTile } from '@/tiles/registry'
import { AddTileMenu } from '@/components/AddTileMenu'

export function GameDashboard() {
  const layouts = useLayoutStore((s) => s.layouts)
  const hidden = useLayoutStore((s) => s.hidden)
  const setLayouts = useLayoutStore((s) => s.setLayouts)
  const reset = useLayoutStore((s) => s.reset)
  const { width, containerRef, mounted } = useContainerWidth()

  const [bp, setBp] = useState<Breakpoint>('lg')
  const visibleIds = visibleTilesFor(bp, hidden)

  // RGL filters layouts to only visible items so missing entries don't crash.
  const filteredLayouts: ResponsiveLayouts = {
    lg: layouts.lg.filter((l) => visibleIds.includes(l.i as TileId)),
    md: layouts.md.filter((l) => visibleIds.includes(l.i as TileId)),
    sm: layouts.sm.filter((l) => visibleIds.includes(l.i as TileId)),
    xs: layouts.xs.filter((l) => visibleIds.includes(l.i as TileId)),
  }

  return (
    <div className="flex flex-col h-full">
      <div className="flex items-center gap-2 px-4 py-2 border-b border-border bg-muted/20">
        <h1 className="text-sm font-semibold tracking-wide uppercase text-muted-foreground">Game</h1>
        <div className="ml-auto flex items-center gap-2">
          <AddTileMenu bp={bp} />
          <Button variant="ghost" size="sm" onClick={reset} className="text-xs">
            Reset Layout
          </Button>
        </div>
      </div>

      <div ref={containerRef} className="flex-1 overflow-auto">
        {mounted && (
          <Responsive
            width={width}
            breakpoints={BREAKPOINTS}
            cols={COLS}
            rowHeight={30}
            margin={[12, 12]}
            containerPadding={[16, 16]}
            layouts={filteredLayouts}
            dragConfig={{ handle: '.tile-drag-handle' }}
            resizeConfig={{ handles: ['se'] }}
            compactor={verticalCompactor}
            onBreakpointChange={(b: string) => setBp(b as Breakpoint)}
            onLayoutChange={(_current: Layout, all: ResponsiveLayouts) => {
              const merged = mergeLayouts(layouts, all)
              setLayouts(merged)
            }}
          >
            {visibleIds.map((id) => (
              <div key={id} className="overflow-hidden">
                {renderTile(id, bp)}
              </div>
            ))}
          </Responsive>
        )}
      </div>
    </div>
  )
}

// RGL only emits layouts for currently rendered (visible) tiles. Merge with
// the existing store so hidden entries keep their last known position.
function mergeLayouts(prev: Record<Breakpoint, LayoutItem[]>, next: ResponsiveLayouts): Record<Breakpoint, LayoutItem[]> {
  const out: Record<Breakpoint, LayoutItem[]> = { ...prev }
  for (const bp of ['lg', 'md', 'sm', 'xs'] as const) {
    const incoming = next[bp]
    if (!incoming) continue
    const incomingIds = new Set(incoming.map((l) => l.i))
    const kept = (prev[bp] ?? []).filter((l) => !incomingIds.has(l.i))
    out[bp] = [...incoming, ...kept]
  }
  return out
}
