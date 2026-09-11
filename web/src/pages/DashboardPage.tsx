import { useState } from "react";
import {
  Activity,
  BarChart3,
  Database,
  FlaskConical,
  Gauge,
  Plus,
  Settings2,
} from "lucide-react";

import { ExperimentTable } from "@/components/dashboard/ExperimentTable";
import { LatencyChart } from "@/components/dashboard/LatencyChart";
import { MetricCard } from "@/components/dashboard/MetricCard";
import { QualityBreakdown } from "@/components/dashboard/QualityBreakdown";
import { Button } from "@/components/ui/button";
import {
  dashboardDataSource,
  initialDashboardSnapshot,
  staticDashboardProfiles,
  type DashboardSnapshot,
  type ReferenceProfile,
} from "@/data/dashboardMockData";

const navigation = [
  { label: "Dashboard", icon: BarChart3, current: true },
  { label: "Experiments", icon: FlaskConical, current: false },
  { label: "Datasets", icon: Database, current: false },
  { label: "Settings", icon: Settings2, current: false },
];

function DashboardPage() {
  const [profiles] = useState<ReferenceProfile[]>(staticDashboardProfiles);
  const [profileId, setProfileId] = useState("reference");
  const [dashboard, setDashboard] = useState<DashboardSnapshot>(
    initialDashboardSnapshot,
  );
  const [showDatasetForm, setShowDatasetForm] = useState(false);
  const [datasetName, setDatasetName] = useState("");
  const [datasetSamples, setDatasetSamples] = useState("");

  const loadDashboard = async (nextProfileId: string) =>
    setDashboard(await dashboardDataSource.getDashboard(nextProfileId));
  const selectProfile = (nextProfileId: string) => {
    setProfileId(nextProfileId);
    void loadDashboard(nextProfileId);
  };

  const addDataset = async () => {
    const samples = Number(datasetSamples);
    if (!datasetName.trim() || !Number.isInteger(samples) || samples < 1)
      return;
    await dashboardDataSource.createDataset(profileId, {
      name: datasetName.trim(),
      samples,
    });
    await loadDashboard(profileId);
    setDatasetName("");
    setDatasetSamples("");
    setShowDatasetForm(false);
  };

  return (
    <main className="min-h-screen bg-[#f8fafc] text-foreground">
      <div className="mx-auto grid min-h-screen max-w-360 lg:grid-cols-[232px_1fr]">
        <aside className="border-b bg-card px-4 py-5 lg:border-r lg:border-b-0">
          <div className="flex items-center gap-3 px-2">
            <div className="grid size-9 place-items-center rounded-lg bg-slate-900 text-white">
              <FlaskConical className="size-5" />
            </div>
            <div>
              <p className="font-semibold tracking-tight">Agentic Lab</p>
              <p className="text-xs text-muted-foreground">
                Research workspace
              </p>
            </div>
          </div>
          <nav className="mt-8 flex gap-1 overflow-x-auto lg:block lg:space-y-1">
            {navigation.map((item) => (
              <button
                key={item.label}
                className={`flex shrink-0 items-center gap-3 rounded-lg px-3 py-2 text-sm font-medium lg:w-full ${item.current ? "bg-slate-900 text-white" : "text-muted-foreground hover:bg-muted hover:text-foreground"}`}
              >
                <item.icon className="size-4" />
                {item.label}
              </button>
            ))}
          </nav>
          <div className="mt-8 hidden rounded-lg border bg-muted/50 p-3 lg:block">
            <p className="text-xs font-medium">Active profile</p>
            <p className="mt-1 text-xs leading-5 text-muted-foreground">
              {dashboard.profile.description}
              <br />
              {dashboard.profile.inference}
            </p>
          </div>
        </aside>
        <div className="min-w-0">
          <header className="flex flex-col gap-4 border-b bg-card px-5 py-4 sm:flex-row sm:items-center sm:justify-between sm:px-8">
            <div>
              <p className="text-xs font-medium uppercase tracking-[0.14em] text-blue-700">
                Benchmark dashboard
              </p>
              <h1 className="mt-1 text-xl font-semibold tracking-tight">
                Incident reporting baseline
              </h1>
            </div>
            <Button size="lg">
              <Plus /> New experiment
            </Button>
          </header>
          <div className="px-5 py-7 sm:px-8">
            <div className="mb-5 flex flex-wrap items-end justify-between gap-4">
              <div>
                <h2 className="text-lg font-semibold">Overview</h2>
                <p className="mt-1 text-sm text-muted-foreground">
                  Warm-run results for the selected profile and dataset.
                </p>
              </div>
              <div className="flex flex-wrap gap-2">
                <label className="grid gap-1 text-xs font-medium text-muted-foreground">
                  Reference profile
                  <select
                    value={profileId}
                    onChange={(event) => selectProfile(event.target.value)}
                    className="h-9 rounded-lg border bg-card px-2 text-sm font-normal text-foreground"
                  >
                    {profiles.map((profile) => (
                      <option key={profile.id} value={profile.id}>
                        {profile.name}
                      </option>
                    ))}
                  </select>
                </label>
                <label className="grid gap-1 text-xs font-medium text-muted-foreground">
                  Dataset
                  <select
                    defaultValue={dashboard.selectedDatasetId}
                    className="h-9 rounded-lg border bg-card px-2 text-sm font-normal text-foreground"
                  >
                    {dashboard.datasets.map((dataset) => (
                      <option key={dataset.id} value={dataset.id}>
                        {dataset.name} · {dataset.revision}
                      </option>
                    ))}
                  </select>
                </label>
              </div>
            </div>
            <section className="grid gap-4 sm:grid-cols-2 xl:grid-cols-4">
              {dashboard.overview.map((metric) => (
                <MetricCard key={metric.label} metric={metric} />
              ))}
            </section>
            <section className="mt-6 grid gap-6 xl:grid-cols-[1.3fr_0.9fr]">
              <article className="rounded-xl border bg-card p-5 shadow-sm">
                <div className="flex items-start justify-between gap-4">
                  <div>
                    <div className="flex items-center gap-2">
                      <Activity className="size-4 text-blue-600" />
                      <h2 className="font-semibold">Response time trend</h2>
                    </div>
                    <p className="mt-1 text-sm text-muted-foreground">
                      Median end-to-end response, including ASR and extraction
                    </p>
                  </div>
                  <span className="rounded-full bg-emerald-50 px-2 py-1 text-xs font-medium text-emerald-700">
                    Profile results
                  </span>
                </div>
                <LatencyChart points={dashboard.latencyTrend} />
              </article>
              <article className="rounded-xl border bg-card p-5 shadow-sm">
                <div className="flex items-center gap-2">
                  <Gauge className="size-4 text-blue-600" />
                  <h2 className="font-semibold">Output quality</h2>
                </div>
                <p className="mt-1 text-sm text-muted-foreground">
                  Structured-output and safety evaluation
                </p>
                <QualityBreakdown metrics={dashboard.quality} />
              </article>
            </section>
            <section className="mt-6 grid gap-6 xl:grid-cols-[1.3fr_0.9fr]">
              <ExperimentTable experiments={dashboard.experiments} />
              <article className="rounded-xl border bg-card p-5 shadow-sm">
                <h2 className="font-semibold">Resource efficiency</h2>
                <p className="mt-1 text-sm text-muted-foreground">
                  Single-user process measurements
                </p>
                <div className="mt-5 space-y-5">
                  {dashboard.resourceMetrics.map((metric) => (
                    <div key={metric.label}>
                      <div className="flex items-center justify-between gap-3 text-sm">
                        <span className="font-medium">{metric.label}</span>
                        <span
                          className={
                            metric.tone === "neutral"
                              ? "text-muted-foreground"
                              : "font-semibold"
                          }
                        >
                          {metric.value}
                        </span>
                      </div>
                      <div className="mt-2 h-2 overflow-hidden rounded-full bg-muted">
                        <div
                          className={
                            metric.tone === "neutral"
                              ? "h-full w-0"
                              : "h-full rounded-full bg-blue-600"
                          }
                          style={{ width: `${metric.percentage}%` }}
                        />
                      </div>
                      <p className="mt-1.5 text-xs text-muted-foreground">
                        Target: {metric.target}
                      </p>
                    </div>
                  ))}
                </div>
              </article>
            </section>
            <section className="mt-6 rounded-xl border bg-card p-5 shadow-sm">
              <div className="flex flex-wrap items-start justify-between gap-3">
                <div>
                  <h2 className="font-semibold">Datasets</h2>
                  <p className="mt-1 text-sm text-muted-foreground">
                    Dataset metadata is scoped to the active profile.
                  </p>
                </div>
                <Button
                  variant="outline"
                  onClick={() => setShowDatasetForm(!showDatasetForm)}
                >
                  <Plus /> Add dataset
                </Button>
              </div>
              {showDatasetForm && (
                <div className="mt-4 flex flex-wrap items-end gap-3 rounded-lg bg-muted/50 p-3">
                  <label className="grid gap-1 text-xs font-medium">
                    Dataset name
                    <input
                      value={datasetName}
                      onChange={(event) => setDatasetName(event.target.value)}
                      className="h-9 rounded-md border bg-card px-2 text-sm"
                      placeholder="e.g. Local accents"
                    />
                  </label>
                  <label className="grid gap-1 text-xs font-medium">
                    Samples
                    <input
                      value={datasetSamples}
                      onChange={(event) =>
                        setDatasetSamples(event.target.value)
                      }
                      className="h-9 w-24 rounded-md border bg-card px-2 text-sm"
                      inputMode="numeric"
                      placeholder="50"
                    />
                  </label>
                  <Button onClick={() => void addDataset()}>
                    Save dataset
                  </Button>
                </div>
              )}
              <div className="mt-4 flex flex-wrap gap-2">
                {dashboard.datasets.map((dataset) => (
                  <span
                    key={dataset.id}
                    className="rounded-lg border bg-muted/40 px-3 py-2 text-sm"
                  >
                    <strong>{dataset.name}</strong>{" "}
                    <span className="text-muted-foreground">
                      {dataset.revision} · {dataset.samples} samples
                    </span>
                  </span>
                ))}
              </div>
            </section>
          </div>
        </div>
      </div>
    </main>
  );
}

export default DashboardPage;
