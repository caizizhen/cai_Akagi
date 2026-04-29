import { Outlet, useLocation } from 'react-router-dom'
import { Sidebar } from '@/components/Sidebar'
import { Statusbar } from '@/components/Statusbar'
import { ErrorBoundary } from '@/components/ErrorBoundary'
import { Toaster } from '@/components/ui/sonner'
import { useTauriBridge } from '@/hooks/useTauriBridge'

export default function App() {
  useTauriBridge()
  const location = useLocation()
  return (
    <>
      <div className="grid grid-cols-[248px_1fr] h-screen bg-background text-foreground">
        <Sidebar />
        <main className="flex flex-col min-w-0 overflow-hidden">
          <div className="flex-1 overflow-auto">
            {/* Keyed by route so a crash on one page doesn't poison the others. */}
            <ErrorBoundary key={location.pathname}>
              <Outlet />
            </ErrorBoundary>
          </div>
          <Statusbar />
        </main>
      </div>
      <Toaster />
    </>
  )
}
