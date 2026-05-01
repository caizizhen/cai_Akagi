// Detail-stats table — mirrors the layout of `libriichi/src/stat.rs`'s
// Display impl, but reads from the in-memory aggregate over the
// frontend-filtered record set.

import { useTranslation } from 'react-i18next'

import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import {
  type AggregateStats,
  avgRank,
  rate,
} from '@/lib/historyStats'

export function StatsTable({ stats }: { stats: AggregateStats }) {
  const { t } = useTranslation()

  const fmtPct = (v: number | null) => (v == null ? '—' : `${(v * 100).toFixed(2)}%`)
  const fmtNum = (v: number | null) => (v == null ? '—' : v.toFixed(1))

  const winRate = rate(stats.agari, stats.rounds)
  const houjuuRate = rate(stats.houjuu, stats.rounds)
  const callRate = rate(stats.fuuro, stats.rounds)
  const riichiRate = rate(stats.riichi, stats.rounds)
  const ryukyokuRate = rate(stats.ryukyoku, stats.rounds)

  const avgWinning = rate(
    stats.agari_point_oya + stats.agari_point_ko,
    stats.agari,
  )
  const avgRiichiWinning = rate(stats.riichi_agari_point, stats.riichi_agari)
  const avgOpenWinning = rate(stats.fuuro_agari_point, stats.fuuro_agari)
  const avgDamaWinning = rate(stats.dama_agari_point, stats.dama_agari)
  const avgWinningTurn = rate(stats.agari_jun, stats.agari)
  const avgDealIn = rate(
    stats.houjuu_point_to_oya + stats.houjuu_point_to_ko,
    stats.houjuu,
  )

  const agariAfterRiichi = rate(stats.riichi_agari, stats.riichi)
  const houjuuAfterRiichi = rate(stats.riichi_houjuu, stats.riichi)
  const agariAfterCall = rate(stats.fuuro_agari, stats.fuuro)
  const houjuuAfterCall = rate(stats.fuuro_houjuu, stats.fuuro)

  const ranksRow = (
    <tr>
      {stats.ranks.map((c, i) => (
        <td key={i} className="px-3 py-1.5">
          <span className="text-muted-foreground mr-1">
            {t(`history.stat.rank${i + 1}`)}
          </span>
          <span className="font-mono">{c}</span>
          <span className="text-muted-foreground ml-1">
            ({fmtPct(rate(c, stats.games))})
          </span>
        </td>
      ))}
    </tr>
  )

  const rows: Array<[string, string]> = [
    [t('history.stat.win_rate'), fmtPct(winRate)],
    [t('history.stat.deal_in_rate'), fmtPct(houjuuRate)],
    [t('history.stat.call_rate'), fmtPct(callRate)],
    [t('history.stat.riichi_rate'), fmtPct(riichiRate)],
    [t('history.stat.ryukyoku_rate'), fmtPct(ryukyokuRate)],
    [t('history.stat.avg_winning'), fmtNum(avgWinning)],
    [t('history.stat.avg_riichi_winning'), fmtNum(avgRiichiWinning)],
    [t('history.stat.avg_open_winning'), fmtNum(avgOpenWinning)],
    [t('history.stat.avg_dama_winning'), fmtNum(avgDamaWinning)],
    [t('history.stat.avg_winning_turn'), fmtNum(avgWinningTurn)],
    [t('history.stat.avg_deal_in'), fmtNum(avgDealIn)],
    [t('history.stat.agari_after_riichi'), fmtPct(agariAfterRiichi)],
    [t('history.stat.houjuu_after_riichi'), fmtPct(houjuuAfterRiichi)],
    [t('history.stat.agari_after_call'), fmtPct(agariAfterCall)],
    [t('history.stat.houjuu_after_call'), fmtPct(houjuuAfterCall)],
    [t('history.stat.yakuman'), `${stats.yakuman}`],
    [t('history.stat.nagashi_mangan'), `${stats.nagashi_mangan}`],
  ]

  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-sm uppercase tracking-wider">
          {t('history.stats')}
        </CardTitle>
      </CardHeader>
      <CardContent>
        <div className="grid grid-cols-1 md:grid-cols-3 gap-x-6 gap-y-1 text-sm">
          <div className="col-span-1 md:col-span-3">
            <table className="w-full">
              <tbody>
                <tr>
                  <td className="px-3 py-1.5 text-muted-foreground">
                    {t('history.stat.games')}
                  </td>
                  <td className="px-3 py-1.5 font-mono">{stats.games}</td>
                  <td className="px-3 py-1.5 text-muted-foreground">
                    {t('history.stat.rounds')}
                  </td>
                  <td className="px-3 py-1.5 font-mono">{stats.rounds}</td>
                  <td className="px-3 py-1.5 text-muted-foreground">
                    {t('history.stat.avg_rank')}
                  </td>
                  <td className="px-3 py-1.5 font-mono">
                    {fmtNum(avgRank(stats))}
                  </td>
                </tr>
                {ranksRow}
              </tbody>
            </table>
          </div>
          {rows.map(([label, value]) => (
            <div
              key={label}
              className="flex items-baseline justify-between py-1 border-b border-border/40"
            >
              <span className="text-muted-foreground">{label}</span>
              <span className="font-mono">{value}</span>
            </div>
          ))}
        </div>
      </CardContent>
    </Card>
  )
}
