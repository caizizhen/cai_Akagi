import { Plus } from 'lucide-react'
import { Button } from '@/components/ui/button'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
  DropdownMenuLabel,
  DropdownMenuSeparator,
} from '@/components/ui/dropdown-menu'
import { useLayoutStore } from '@/stores/layoutStore'
import { TILE_TITLES, type Breakpoint, type TileId } from '@/tiles/defaults'

// Stable empty fallback — see OpponentsTile note on Zustand selector identity.
const EMPTY_HIDDEN: readonly TileId[] = []

export function AddTileMenu({ bp }: { bp: Breakpoint }) {
  const hidden = useLayoutStore((s) => s.hidden[bp] ?? EMPTY_HIDDEN)
  const mode = useLayoutStore((s) => s.mode)
  const show = useLayoutStore((s) => s.show)

  // 3p: never offer player-3 in the Add menu.
  const offered = mode === '3p' ? hidden.filter((id) => id !== 'player-3') : hidden

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button variant="outline" size="sm" className="gap-1.5 text-xs">
          <Plus className="h-3.5 w-3.5" />
          Add tile
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        <DropdownMenuLabel className="text-xs">Hidden tiles</DropdownMenuLabel>
        <DropdownMenuSeparator />
        {offered.length === 0 ? (
          <DropdownMenuItem disabled className="text-xs text-muted-foreground">
            All tiles are visible.
          </DropdownMenuItem>
        ) : offered.map((id) => (
          <DropdownMenuItem key={id} onClick={() => show(id, bp)} className="text-xs">
            {TILE_TITLES[id]}
          </DropdownMenuItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
