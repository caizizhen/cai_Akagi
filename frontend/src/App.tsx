import { Route, Routes } from 'react-router-dom'
import { Sidebar } from '@/components/Sidebar'
import { Statusbar } from '@/components/Statusbar'
import { useTauriBridge } from '@/hooks/useTauriBridge'
import { Overview } from '@/routes/Overview'
import { GameDashboard } from '@/routes/GameDashboard'
import { Bots } from '@/routes/Bots'
import { Logs } from '@/routes/Logs'
import { Settings } from '@/routes/Settings'

export default function App() {
  useTauriBridge()
  return (
    <div className="grid grid-cols-[248px_1fr] h-screen bg-background text-foreground">
      <Sidebar />
      <main className="flex flex-col min-w-0 overflow-hidden">
        <div className="flex-1 overflow-auto">
          <Routes>
            <Route path="/" element={<Overview />} />
            <Route path="/game" element={<GameDashboard />} />
            <Route path="/bots" element={<Bots />} />
            <Route path="/logs" element={<Logs />} />
            <Route path="/settings" element={<Settings />} />
          </Routes>
        </div>
        <Statusbar />
      </main>
    </div>
  )
}
