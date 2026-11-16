import { useMemo, useState } from 'react';
import { projectObservations } from '../../../shared/phone-benchmarks/index.ts';
import { allProjection, conversationID, fixtureTime, snapshotFor } from './fixtures';

export function usePhoneResults(scope: { conversation_id?: string; sweep_id?: string; configuration_id?: string } = {}) {
  const [selectedId, selectConversation] = useState<string | null>(scope.conversation_id ?? (scope.sweep_id ? null : conversationID));
  const [source, changeSource] = useState<'manual' | 'sweep' | 'acceptance'>(scope.sweep_id ? 'sweep' : 'manual');
  const [search, setSearch] = useState('');
  const [historyPeriod, setHistoryPeriod] = useState<'all' | 'today' | 'week'>('all');
  const history = useMemo(() => {
    const items = allProjection.conversations.filter(item => (scope.sweep_id ? item.context.sweep_id === scope.sweep_id : item.source === source))
      .map(item => ({ entity_id: item.entity_id, source: item.source, status: item.status, started_at_ms: item.started_at_ms!,
        configuration_id: item.context.configuration_id!, asr_name: 'Whisper Small', reasoning_name: 'Qwen3.5 4B' }))
      .filter(item => `${item.entity_id} ${item.asr_name} ${item.reasoning_name}`.toLowerCase().includes(search.toLowerCase()))
      .filter(item => historyPeriod === 'all' || item.started_at_ms >= fixtureTime - (historyPeriod === 'today' ? 0 : 7 * 86_400_000));
    return { items, total: items.length, next_cursor: null };
  }, [source, search, scope.sweep_id, historyPeriod]);
  const variant = new URLSearchParams(location.search).get('fixture') ?? undefined;
  const snapshot = useMemo(() => snapshotFor(selectedId, scope.sweep_id, variant), [selectedId, scope.sweep_id, variant]);
  const projection = useMemo(() => projectObservations(snapshot.records, { filtered: true, selected_record_keys: snapshot.selected_record_keys }), [snapshot]);
  return { snapshot, projection, history, selectedId, selectConversation, source,
    setSource: (value: typeof source) => { changeSource(value); selectConversation(value === 'sweep' ? 'illustrative-sweep-chat' : value === 'manual' ? conversationID : null); },
    search, setSearch, historyPeriod, setHistoryPeriod, plannedSweep: null, visible: true, loadMore() {}, refresh() {}, error: '', loading: false, updatedAt: fixtureTime + 60_000 };
}
