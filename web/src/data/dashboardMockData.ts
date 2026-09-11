export type MetricTone = 'good' | 'watch' | 'neutral'
export type DashboardMetric = { label: string; value: string; target: string; detail: string; tone: MetricTone }
export type TrendPoint = { label: string; latency: number; target: number }
export type QualityMetric = { name: string; value: number; target: number; unit: '%'; higherIsBetter: boolean }
export type ResourceMetric = { label: string; value: string; target: string; percentage: number; tone: MetricTone }
export type Dataset = { id: string; name: string; samples: number; revision: string }
export type Experiment = { id: string; model: string; quantization: string; dataset: string; device: string; status: 'Completed' | 'Running' | 'Queued'; latency: string; quality: string; timestamp: string }
export type ReferenceProfile = { id: string; name: string; description: string; cpuCores: number; memoryGB: number; inference: string }
export type DashboardSnapshot = { profile: ReferenceProfile; period: string; updatedAt: string; datasets: Dataset[]; selectedDatasetId: string; overview: DashboardMetric[]; latencyTrend: TrendPoint[]; quality: QualityMetric[]; resourceMetrics: ResourceMetric[]; experiments: Experiment[] }

export interface DashboardDataSource {
  getProfiles(): Promise<ReferenceProfile[]>
  getDashboard(profileId: string, datasetId?: string): Promise<DashboardSnapshot>
  createDataset(profileId: string, dataset: Omit<Dataset, 'id' | 'revision'>): Promise<Dataset>
}

const days = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun']
const trend = (values: number[]) => values.map((latency, index) => ({ label: days[index], latency, target: 10 }))
const quality = (f1: number, hallucination: number): QualityMetric[] => [
  { name: 'Schema conformance', value: 99.2, target: 99, unit: '%', higherIsBetter: true },
  { name: 'Field extraction F1', value: f1, target: 90, unit: '%', higherIsBetter: true },
  { name: 'Clarification trigger', value: 96.1, target: 95, unit: '%', higherIsBetter: true },
  { name: 'Hallucination rate', value: hallucination, target: 5, unit: '%', higherIsBetter: false },
]

const profiles: Record<string, DashboardSnapshot> = {
  reference: {
    profile: { id: 'reference', name: 'Reference CPU', description: '4-core CPU · 8 GB RAM', cpuCores: 4, memoryGB: 8, inference: 'CPU-only inference' }, period: 'Last 7 days', updatedAt: 'Updated 12 min ago', selectedDatasetId: 'incident-v03',
    datasets: [{ id: 'incident-v03', name: 'Incident reporting', samples: 60, revision: 'v0.3' }, { id: 'noisy-v01', name: 'Noisy field speech', samples: 36, revision: 'v0.1' }],
    overview: [{ label: 'Experiments completed', value: '24', target: '4 this week', detail: '+6 vs. prior week', tone: 'good' }, { label: 'End-to-end response', value: '7.8s', target: '< 10s target', detail: 'Median across warm runs', tone: 'good' }, { label: 'Task success', value: '91.6%', target: '≥ 90% target', detail: '55 of 60 scripted tasks', tone: 'good' }, { label: 'Energy / completed report', value: 'Unavailable', target: 'Instrumentation pending', detail: 'Not recorded as zero', tone: 'neutral' }],
    latencyTrend: trend([8.7, 8.1, 7.5, 8.3, 7.2, 7.8, 7.8]), quality: quality(91.4, 3.4), resourceMetrics: [{ label: 'Peak RAM', value: '4.8 GB', target: '< 6 GB', percentage: 80, tone: 'good' }, { label: 'Average CPU', value: '72%', target: '< 80%', percentage: 72, tone: 'good' }, { label: 'ASR real-time factor', value: '0.42', target: '< 0.5', percentage: 84, tone: 'good' }, { label: 'Sustained operation', value: 'Unavailable', target: '≥ 8 hours', percentage: 0, tone: 'neutral' }],
    experiments: [{ id: 'EXP-024', model: 'Qwen 2.5 1.5B', quantization: 'Q4_K_M', dataset: 'Incident v0.3', device: 'Reference CPU', status: 'Completed', latency: '7.8s', quality: '91.6%', timestamp: 'Today, 10:42' }, { id: 'EXP-023', model: 'Llama 3.2 1B', quantization: 'Q4_K_M', dataset: 'Incident v0.3', device: 'Reference CPU', status: 'Completed', latency: '8.5s', quality: '88.2%', timestamp: 'Yesterday, 16:18' }],
  },
  mobile: {
    profile: { id: 'mobile', name: 'Mobile baseline', description: '6-core mobile CPU · 6 GB RAM', cpuCores: 6, memoryGB: 6, inference: 'On-device CPU inference' }, period: 'Last 7 days', updatedAt: 'Updated 18 min ago', selectedDatasetId: 'incident-v03',
    datasets: [{ id: 'incident-v03', name: 'Incident reporting', samples: 48, revision: 'v0.3' }, { id: 'sg-terms-v01', name: 'Singapore terminology', samples: 28, revision: 'v0.1' }],
    overview: [{ label: 'Experiments completed', value: '18', target: '4 this week', detail: '+2 vs. prior week', tone: 'good' }, { label: 'End-to-end response', value: '9.4s', target: '< 10s target', detail: 'Median across warm runs', tone: 'good' }, { label: 'Task success', value: '89.6%', target: '≥ 90% target', detail: '43 of 48 scripted tasks', tone: 'watch' }, { label: 'Energy / completed report', value: '3.2 Wh', target: 'Baseline reading', detail: 'Whole-device measurement', tone: 'good' }],
    latencyTrend: trend([10.1, 9.8, 9.6, 9.2, 9.4, 9.1, 9.4]), quality: quality(89.8, 4.2), resourceMetrics: [{ label: 'Peak RAM', value: '4.4 GB', target: '< 4.5 GB', percentage: 98, tone: 'watch' }, { label: 'Average CPU', value: '78%', target: '< 80%', percentage: 78, tone: 'good' }, { label: 'ASR real-time factor', value: '0.48', target: '< 0.5', percentage: 96, tone: 'watch' }, { label: 'Sustained operation', value: '6.7 hours', target: '≥ 8 hours', percentage: 84, tone: 'watch' }],
    experiments: [{ id: 'EXP-018', model: 'Qwen 2.5 1.5B', quantization: 'Q4_K_M', dataset: 'Incident v0.3', device: 'Mobile baseline', status: 'Completed', latency: '9.4s', quality: '89.6%', timestamp: 'Today, 09:14' }, { id: 'EXP-017', model: 'Llama 3.2 1B', quantization: 'Q4_K_M', dataset: 'Singapore terms v0.1', device: 'Mobile baseline', status: 'Completed', latency: '8.9s', quality: '90.2%', timestamp: 'Yesterday, 14:06' }],
  },
  laptop: {
    profile: { id: 'laptop', name: 'Laptop baseline', description: '8-core CPU · 16 GB RAM', cpuCores: 8, memoryGB: 16, inference: 'CPU-only inference' }, period: 'Last 7 days', updatedAt: 'Updated 7 min ago', selectedDatasetId: 'incident-v03',
    datasets: [{ id: 'incident-v03', name: 'Incident reporting', samples: 80, revision: 'v0.3' }, { id: 'corrections-v01', name: 'Corrections & retrieval', samples: 42, revision: 'v0.1' }],
    overview: [{ label: 'Experiments completed', value: '31', target: '4 this week', detail: '+8 vs. prior week', tone: 'good' }, { label: 'End-to-end response', value: '5.6s', target: '< 10s target', detail: 'Median across warm runs', tone: 'good' }, { label: 'Task success', value: '94.8%', target: '≥ 90% target', detail: '76 of 80 scripted tasks', tone: 'good' }, { label: 'Energy / completed report', value: '1.8 Wh', target: 'Baseline reading', detail: 'Measured at wall power', tone: 'good' }],
    latencyTrend: trend([6.4, 6.0, 5.8, 5.4, 5.5, 5.7, 5.6]), quality: quality(93.6, 2.5), resourceMetrics: [{ label: 'Peak RAM', value: '5.2 GB', target: '< 12 GB', percentage: 43, tone: 'good' }, { label: 'Average CPU', value: '56%', target: '< 80%', percentage: 56, tone: 'good' }, { label: 'ASR real-time factor', value: '0.29', target: '< 0.5', percentage: 58, tone: 'good' }, { label: 'Sustained operation', value: '8.4 hours', target: '≥ 8 hours', percentage: 100, tone: 'good' }],
    experiments: [{ id: 'EXP-031', model: 'Gemma 2 2B', quantization: 'Q5_K_M', dataset: 'Incident v0.3', device: 'Laptop baseline', status: 'Completed', latency: '5.6s', quality: '94.8%', timestamp: 'Today, 11:08' }, { id: 'EXP-030', model: 'Qwen 2.5 3B', quantization: 'Q4_K_M', dataset: 'Corrections v0.1', device: 'Laptop baseline', status: 'Completed', latency: '6.2s', quality: '93.9%', timestamp: 'Yesterday, 15:34' }],
  },
}

const clone = <T,>(value: T) => structuredClone(value)
export const staticDashboardProfiles = Object.values(profiles).map(({ profile }) => clone(profile))
export const initialDashboardSnapshot = clone(profiles.reference)
export const dashboardDataSource: DashboardDataSource = {
  async getProfiles() { return clone(staticDashboardProfiles) },
  async getDashboard(profileId) { return clone(profiles[profileId] ?? profiles.reference) },
  async createDataset(profileId, dataset) { const profile = profiles[profileId] ?? profiles.reference; const newDataset = { ...dataset, id: `dataset-${crypto.randomUUID()}`, revision: 'Draft' }; profile.datasets.push(newDataset); return clone(newDataset) },
}
