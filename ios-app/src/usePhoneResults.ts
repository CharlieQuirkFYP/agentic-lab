import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { AppState } from 'react-native';
import { useFocusEffect } from 'expo-router';
import { openDatabaseSync, type SQLiteDatabase } from 'expo-sqlite';
import { projectObservations } from '../../shared/phone-benchmarks/index';
import type { ObservationProjection } from '../../shared/phone-benchmarks/types';
import type { ResultsScope } from './benchmarkStore';
import type { BenchmarkSweep } from './benchmarkRunner';
import { Benchmark } from './phoneMetrics';
import { subscribeResultsRefresh } from './resultsRefresh';

type ResultsSource = 'manual' | 'sweep' | 'acceptance';
type Snapshot = ReturnType<typeof Benchmark.store.resultsSnapshot>;
type History = ReturnType<typeof Benchmark.store.resultsHistory>;
type HistoryPeriod = 'all' | 'today' | 'week';
let planDB: SQLiteDatabase | null = null;
function readSweepPlan(id: string): BenchmarkSweep | null {
  planDB ??= openDatabaseSync('benchmark-plans.db');
  if (!planDB.getFirstSync("SELECT name FROM sqlite_master WHERE type='table' AND name='benchmark_sweeps'")) return null;
  const row = planDB.getFirstSync<{ payload: string }>('SELECT payload FROM benchmark_sweeps WHERE id=?', id);
  return row ? JSON.parse(row.payload) as BenchmarkSweep : null;
}

/** The Results route reads saved observations; the root remains the only collector. */
export function usePhoneResults(scopeInput: ResultsScope = {}) {
  const { conversation_id, sweep_id, configuration_id } = scopeInput;
  const [selectedId, setSelectedId] = useState<string | null>(conversation_id ?? null);
  const [source, setSourceState] = useState<ResultsSource>(sweep_id ? 'sweep' : 'manual');
  const [search, setSearchState] = useState('');
  const [historyPeriod, setHistoryPeriod] = useState<HistoryPeriod>('all');
  const [plannedSweep, setPlannedSweep] = useState<BenchmarkSweep | null>(null);
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [projection, setProjection] = useState<ObservationProjection | null>(null);
  const [history, setHistory] = useState<History | null>(null);
  const [error, setError] = useState('');
  const [loading, setLoading] = useState(true);
  const [updatedAt, setUpdatedAt] = useState(0);
  const [active, setActive] = useState(AppState.currentState === 'active');
  const [focused, setFocused] = useState(false);
  const since_ms = useMemo(() => historyPeriod === 'today' ? new Date().setHours(0, 0, 0, 0)
    : historyPeriod === 'week' ? Date.now() - 7 * 86400000 : undefined, [historyPeriod, focused, active]);
  const previousSnapshot = useRef('');
  const historyKey = useRef('');
  const latestHistory = useRef<History | null>(null);

  useFocusEffect(useCallback(() => {
    setFocused(true);
    return () => setFocused(false);
  }, []));
  useEffect(() => {
    const subscription = AppState.addEventListener('change', state => setActive(state === 'active'));
    return () => subscription.remove();
  }, []);
  useEffect(() => {
    setSelectedId(conversation_id ?? null);
    setSourceState(sweep_id ? 'sweep' : 'manual');
    setSnapshot(null); setProjection(null); previousSnapshot.current = '';
    setPlannedSweep(null);
  }, [conversation_id, sweep_id]);

  const refresh = useCallback(() => {
    if (!focused || !active) return;
    try {
      const nextHistory = Benchmark.store.resultsHistory({ source: sweep_id ? undefined : source, sweep_id, query: search, since_ms, limit: 20 });
      const key = JSON.stringify([source, sweep_id, search, since_ms]);
      const previous = historyKey.current === key ? latestHistory.current : null;
      // Preserve already paged history while refreshing its first page. Switching
      // filters starts a new page. The total is always the complete filtered count.
      const firstIDs = new Set(nextHistory.items.map(item => item.entity_id));
      const extras = previous && previous.items.length > 20
        ? previous.items.slice(20).filter(item => !firstIDs.has(item.entity_id)) : [];
      const samePage = previous?.items.slice(0, 20).every((item, index) => item.entity_id === nextHistory.items[index]?.entity_id);
      const merged = extras.length && samePage && nextHistory.total === previous!.total
        ? { ...nextHistory, items: [...nextHistory.items, ...extras], next_cursor: previous!.next_cursor }
        : nextHistory;
      historyKey.current = key;
      latestHistory.current = merged;
      setHistory(merged);
      const latest = !sweep_id && !selectedId && (search || since_ms !== undefined)
        ? Benchmark.store.resultsHistory({ source, limit: 1 }).items[0] : nextHistory.items[0];
      const id = selectedId ?? (sweep_id ? null : latest?.entity_id ?? null);
      // An explicit sweep stays at sweep scope until a conversation is selected.
      const scope = id ? { conversation_id: id, ...(sweep_id ? { sweep_id } : {}), ...(configuration_id ? { configuration_id } : {}) }
        : sweep_id ? { sweep_id, ...(configuration_id ? { configuration_id } : {}) } : null;
      if (!scope) {
        previousSnapshot.current = '';
        setSnapshot(null); setProjection(null);
      } else {
        const next = Benchmark.store.resultsSnapshot(scope);
        const identity = JSON.stringify([scope, next.as_of]);
        if (identity !== previousSnapshot.current) {
          const projected = projectObservations(next.records, { filtered: true, selected_record_keys: next.selected_record_keys });
          setPlannedSweep(sweep_id ? readSweepPlan(sweep_id) : null);
          previousSnapshot.current = identity;
          setSnapshot(next); setProjection(projected); setUpdatedAt(Date.now());
        }
      }
      setError('');
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : 'Could not read saved results.');
    } finally { setLoading(false); }
  }, [active, focused, source, sweep_id, configuration_id, selectedId, search, since_ms]);

  useEffect(() => {
    if (!focused || !active) return;
    return subscribeResultsRefresh({ subscribe: Benchmark.store.subscribe, refresh });
  }, [focused, active, refresh]);

  const selectConversation = useCallback((id: string) => {
    setSelectedId(id || null); setLoading(true);
    setSnapshot(null); setProjection(null); previousSnapshot.current = '';
  }, []);
  const setSource = useCallback((value: ResultsSource) => {
    setSourceState(value); setSelectedId(null); setLoading(true);
    setSnapshot(null); setProjection(null); previousSnapshot.current = '';
  }, []);
  const setSearch = useCallback((value: string) => setSearchState(value), []);
  const loadMore = useCallback(() => {
    if (!focused || !active || !latestHistory.current?.next_cursor) return;
    try {
      const next = Benchmark.store.resultsHistory({ source: sweep_id ? undefined : source, sweep_id, query: search, since_ms, limit: 20, cursor: latestHistory.current.next_cursor });
      const previous = latestHistory.current;
      const ids = new Set(previous.items.map(item => item.entity_id));
      const merged = { ...next, items: [...previous.items, ...next.items.filter(item => !ids.has(item.entity_id))] };
      latestHistory.current = merged; setHistory(merged); setError('');
    } catch (cause) { setError(cause instanceof Error ? cause.message : 'Could not load more results.'); }
  }, [focused, active, source, sweep_id, search, since_ms]);
  return { snapshot, projection, history, selectedId, selectConversation, source, setSource, search, setSearch, historyPeriod, setHistoryPeriod, plannedSweep, loadMore, refresh, error, loading, updatedAt, visible: focused && active };
}
