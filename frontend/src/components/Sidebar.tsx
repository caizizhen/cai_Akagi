import { NavLink } from 'react-router-dom'
import { useTranslation } from 'react-i18next'
import {
  LayoutDashboard,
  Gamepad2,
  Bot,
  ScrollText,
  Settings as SettingsIcon,
} from 'lucide-react'
import { cn } from '@/lib/utils'
import { LANG_LABELS, SUPPORTED_LANGS, type SupportedLang } from '@/i18n'

type NavItem = { to: string; key: string; Icon: typeof Bot }

const NAV: NavItem[] = [
  { to: '/',         key: 'nav.overview', Icon: LayoutDashboard },
  { to: '/game',     key: 'nav.game',     Icon: Gamepad2 },
  { to: '/bots',     key: 'nav.bots',     Icon: Bot },
  { to: '/logs',     key: 'nav.logs',     Icon: ScrollText },
  { to: '/settings', key: 'nav.settings', Icon: SettingsIcon },
]

export function Sidebar() {
  const { t, i18n } = useTranslation()
  return (
    <aside className="flex flex-col gap-4 px-4 py-5 border-r border-border bg-sidebar text-sidebar-foreground overflow-y-auto">
      <Brand />
      <nav className="flex flex-col gap-0.5">
        {NAV.map(({ to, key, Icon }) => (
          <NavLink
            key={to}
            to={to}
            end={to === '/'}
            className={({ isActive }) =>
              cn(
                'flex items-center gap-2.5 px-3 py-2 rounded-md text-sm transition-colors',
                'hover:bg-sidebar-accent hover:text-sidebar-accent-foreground',
                isActive && 'bg-sidebar-accent text-sidebar-accent-foreground font-medium',
              )
            }
          >
            <Icon className="h-4 w-4" />
            <span>{t(key)}</span>
          </NavLink>
        ))}
      </nav>

      <div className="mt-auto flex items-center justify-between text-xs text-muted-foreground pt-2 border-t border-border">
        <span>v3.0.0</span>
        <select
          className="bg-transparent border border-border rounded px-1.5 py-0.5"
          value={i18n.language}
          onChange={(e) => void i18n.changeLanguage(e.target.value)}
        >
          {SUPPORTED_LANGS.map((lang) => (
            <option key={lang} value={lang}>{LANG_LABELS[lang as SupportedLang]}</option>
          ))}
        </select>
      </div>
    </aside>
  )
}

function Brand() {
  return (
    <div className="flex items-center gap-3 px-1 pb-2">
      <svg viewBox="0 0 32 32" width="28" height="28" aria-hidden="true">
        <defs>
          <linearGradient id="logoGrad" x1="0" y1="0" x2="1" y2="1">
            <stop offset="0%" stopColor="#34d399" />
            <stop offset="100%" stopColor="#0ea5a4" />
          </linearGradient>
        </defs>
        <path d="M4 26 L16 4 L28 26 L22 26 L16 14 L10 26 Z" fill="url(#logoGrad)" />
        <path d="M12 22 L20 22 L20 24 L12 24 Z" fill="#0a1116" />
      </svg>
      <div className="flex items-baseline gap-1.5">
        <span className="font-semibold text-base">Akagi</span>
        <span className="text-xs text-muted-foreground">V3</span>
      </div>
    </div>
  )
}
