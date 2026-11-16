import { fixtureRecords, fixtureTime } from './fixtures';

let settings = { url: '', enabled: false, pending: 0, blocked: 0, error: '', lastUpload: fixtureTime, revision: 0 };
const listeners = new Set<() => void>();
const pins = new Set<string>();
export const Metrics = {
  subscribe(listener: () => void) { listeners.add(listener); return () => { listeners.delete(listener); }; },
  getSnapshot: () => settings,
  configure(url: string, enabled: boolean) { settings = { ...settings, url, enabled, revision: settings.revision + 1 }; listeners.forEach(listener => listener()); },
};
export const Benchmark = {
  async flush() {}, getError: () => '',
  store: {
    resultsStatus: () => ({ pending: 0, blocked: 0, lastUpload: fixtureTime }),
    stats: () => ({ records: fixtureRecords.length, bytes: JSON.stringify(fixtureRecords).length, pending: 0, blocked: 0, lastUpload: fixtureTime }),
    isPinned: (id: string) => pins.has(id),
    pin(id: string, value: boolean) { if (value) pins.add(id); else pins.delete(id); },
    removeAcknowledgedHistory() {}, retryBlocked() {},
  },
};
