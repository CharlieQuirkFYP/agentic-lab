interface Event {
  name: string; unit: string; scope: string; source: string;
  value: number | string | boolean | null;
}
interface Batch { events: Event[] }
export interface ObservationGroup {
  key: string; name: string; unit: string; scope: string; source: string;
  mean: number | null; count: number; unavailable: number;
}

export function summarizeObservations(batches: Batch[]): ObservationGroup[] {
  const groups = new Map<string, ObservationGroup>();
  for (const batch of batches) for (const event of batch.events) {
    const key = JSON.stringify([event.name, event.unit, event.scope, event.source]);
    const group = groups.get(key) ?? { key, name: event.name, unit: event.unit,
      scope: event.scope, source: event.source, mean: null, count: 0, unavailable: 0 };
    if (typeof event.value === 'number' && Number.isFinite(event.value)) {
      group.count++;
      group.mean = group.mean === null ? event.value :
        group.mean * ((group.count - 1) / group.count) + event.value / group.count;
    } else if (event.value === null) group.unavailable++;
    groups.set(key, group);
  }
  return [...groups.values()].filter(group => group.count > 0 || group.unavailable > 0);
}
