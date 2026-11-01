import { createBrowserRouter, Navigate } from "react-router-dom";

import DashboardPage from "@/pages/DashboardPage";
import BenchmarkTestingPage from "@/pages/BenchmarkTestingPage";
import DownloadsPage from "@/pages/DownloadsPage";
import ExperimentDetailsPage from "@/pages/ExperimentDetailsPage";

export const router = createBrowserRouter([
  { path: "/", element: <Navigate to="/dashboard" replace /> },
  {
    path: "/dashboard",
    element: <DashboardPage />,
  },
  {
    path: "/experiments/:id",
    element: <ExperimentDetailsPage />,
  },
  {
    path: "/benchmark-testing",
    element: <BenchmarkTestingPage />,
  },
  { path: "/downloads", element: <DownloadsPage /> },
  { path: "*", element: <Navigate to="/dashboard" replace /> },
]);
