import { useTranslation } from 'react-i18next'
import { TileFrame } from '@/components/TileFrame'
import { useAnalysisStore } from '@/stores/analysisStore'
import { TILE_LABELS_34 } from '@/lib/tileIdx'
import type { Breakpoint } from '@/tiles/defaults'

export function RiskChartTile({ bp }: { bp: Breakpoint }) {
  const { t } = useTranslation()
  const risk = useAnalysisStore((s) => s.result?.mixed_risk ?? null)

  return (
    <TileFrame id="risk-chart" title={t('tile.risk_chart')} bp={bp} contentClassName="flex flex-col gap-1.5">
      {!risk ? (
        <span className="text-muted-foreground text-sm">{t('tile.risk_chart_empty')}</span>
      ) : (
        <div className="grid grid-cols-[auto_1fr_auto] gap-x-2 gap-y-0.5 text-[10px] font-mono">
          {risk.map((v, i) => (
            <Row key={i} label={TILE_LABELS_34[i]} value={v} />
          ))}
        </div>
      )}
    </TileFrame>
  )
}

function Row({ label, value }: { label: string; value: number }) {
  const v = value ?? 0
  const w = Math.min(100, Math.max(0, v))
  const color = v >= 20 ? 'bg-red-500' : v >= 10 ? 'bg-amber-500' : 'bg-emerald-500'
  return (
    <>
      <span className="text-muted-foreground">{label}</span>
      <span className="bg-muted/40 rounded h-3 overflow-hidden flex">
        <span className={`${color} h-full`} style={{ width: `${w}%` }} />
      </span>
      <span className="text-right">{v.toFixed(1)}</span>
    </>
  )
}
