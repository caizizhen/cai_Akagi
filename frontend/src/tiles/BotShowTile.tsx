import { useMemo, useRef } from 'react'
import { useTranslation } from 'react-i18next'
import { TileFrame } from '@/components/TileFrame'
import { Mahgen } from '@/components/Mahgen'
import { useNotifyStore } from '@/stores/notifyStore'
import { mjaiToMahgen } from '@/lib/tileIdx'
import type { Breakpoint } from '@/tiles/defaults'
import type { ShowItem, ShowMeta } from '@/types'

// "#aabbcc" → "rgba(170,187,204,a)". Mirrors the helper in BotActionTile.tsx.
function hexToRgba(hex: string | undefined, alpha: number): string | undefined {
  if (!hex) return undefined
  const m = /^#?([0-9a-f]{6})$/i.exec(hex.trim())
  if (!m) return undefined
  const n = parseInt(m[1], 16)
  return `rgba(${(n >> 16) & 0xff}, ${(n >> 8) & 0xff}, ${n & 0xff}, ${alpha})`
}

function pickShow(meta: unknown): ShowMeta | null {
  if (!meta || typeof meta !== 'object') return null
  const show = (meta as Record<string, unknown>).show
  if (!show || typeof show !== 'object') return null
  const items = (show as Record<string, unknown>).items
  if (!Array.isArray(items) || items.length === 0) return null
  return show as ShowMeta
}

function hasContent(it: ShowItem): boolean {
  return Boolean(it.label || it.tiles || (it.pais && it.pais.length))
}

export function BotShowTile({ bp }: { bp: Breakpoint }) {
  const { t } = useTranslation()
  const responses = useNotifyStore((s) => s.responses)

  const show = useMemo(() => {
    for (let i = responses.length - 1; i >= 0; i--) {
      const s = pickShow(responses[i].meta)
      if (s) return s
    }
    return null
  }, [responses])

  const rowRef = useRef<HTMLOListElement>(null)
  const items = show?.items.filter(hasContent) ?? []

  return (
    <TileFrame
      id="bot-show"
      title={show?.title ?? t('tile.bot_show_default_title')}
      bp={bp}
      contentClassName="p-2"
    >
      {items.length === 0 ? (
        <span className="text-muted-foreground text-sm px-1">{t('tile.bot_show_empty')}</span>
      ) : (
        <ol ref={rowRef} className="flex flex-col gap-1">
          {items.map((it, i) => {
            const seq = it.tiles ?? (it.pais ? mjaiToMahgen(it.pais) : '')
            return (
              <li
                key={i}
                className="flex items-center gap-2 rounded-md border border-border px-2 py-1.5"
                style={{
                  backgroundColor: hexToRgba(it.color, 0.1),
                  borderLeftColor: it.color,
                  borderLeftWidth: it.color ? 3 : undefined,
                }}
              >
                {seq && (
                  <Mahgen seq={seq} kind="bot-show" containerRef={rowRef} />
                )}
                <div className="flex flex-col flex-1 min-w-0">
                  {it.label && (
                    <span className="text-sm text-foreground truncate">{it.label}</span>
                  )}
                  {it.note && (
                    <span className="text-[10px] text-muted-foreground truncate">{it.note}</span>
                  )}
                </div>
                {it.value && (
                  <span className="text-xs font-mono tabular-nums text-foreground/90">
                    {it.value}
                  </span>
                )}
              </li>
            )
          })}
        </ol>
      )}
    </TileFrame>
  )
}
