import { createBrowserRouter } from 'react-router-dom'

import DashboardPage from '@/pages/DashboardPage'
import BenchmarkTestingPage from '@/pages/BenchmarkTestingPage'
import DownloadsPage from '@/pages/DownloadsPage'

export const router = createBrowserRouter([
  {
    path: '/',
    element: <DashboardPage />,
  },
  {
    path: '/benchmark-testing',
    element: <BenchmarkTestingPage />,
  },
  {
    path: '/downloads',
    element: <DownloadsPage />,
  },
])
