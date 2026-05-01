import { useTranslation } from 'react-i18next'

export function Statusbar() {
  const { t } = useTranslation()
  return (
    <footer className="flex items-center justify-between border-t border-border px-4 py-1.5 text-xs text-muted-foreground bg-muted/30">
      <span className="flex items-center gap-1.5">
        <span className="h-1.5 w-1.5 rounded-full bg-emerald-400" />
        <span>{t('status.connected')}</span>
      </span>
      <span className="flex items-center gap-3">
        <span className="flex items-center gap-1.5">
          <span className="h-1.5 w-1.5 rounded-full bg-emerald-400" />
          {t('status.events_live')}
        </span>
        <span className="flex items-center gap-1.5">
          <span className="h-1.5 w-1.5 rounded-full bg-emerald-400" />
          {t('status.analysis_live')}
        </span>
      </span>
    </footer>
  )
}
