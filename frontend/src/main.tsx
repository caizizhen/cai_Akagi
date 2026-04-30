import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { createHashRouter, RouterProvider } from 'react-router-dom'
import 'mahgen'
import './index.css'
import './i18n'
import App from './App.tsx'
import { Overview } from '@/routes/Overview'
import { GameDashboard } from '@/routes/GameDashboard'
import { Bots } from '@/routes/Bots'
import { Logs } from '@/routes/Logs'
import { Settings } from '@/routes/Settings'

const router = createHashRouter([
  {
    element: <App />,
    children: [
      { index: true, element: <Overview /> },
      { path: 'game', element: <GameDashboard /> },
      { path: 'bots', element: <Bots /> },
      { path: 'logs', element: <Logs /> },
      { path: 'settings', element: <Settings /> },
    ],
  },
])

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <RouterProvider router={router} />
  </StrictMode>,
)
