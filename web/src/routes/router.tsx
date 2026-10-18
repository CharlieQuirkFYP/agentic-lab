import { createBrowserRouter, Navigate } from "react-router-dom";

import ConsoleLayout from "@/components/ConsoleLayout";
import BenchmarkPage from "@/pages/BenchmarkPage";
import HomePage from "@/pages/HomePage";

export const router = createBrowserRouter([
  {
    path: "/",
    element: <ConsoleLayout />,
    children: [
      { index: true, element: <Navigate to="/voice" replace /> },
      { path: "voice", element: <HomePage /> },
      { path: "benchmarks", element: <BenchmarkPage /> },
      { path: "*", element: <Navigate to="/voice" replace /> },
    ],
  },
]);
