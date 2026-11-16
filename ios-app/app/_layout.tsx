import { Stack } from "expo-router";
import { useEffect } from "react";
import { startMetricsSync } from "../src/phoneMetrics";

export default function Layout() {
  useEffect(startMetricsSync, []);
  return <Stack screenOptions={{ headerShown: false }} />;
}
