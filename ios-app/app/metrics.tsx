import { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import { Alert, Pressable, ScrollView, Share, Text, TextInput, View } from 'react-native';
import { Link, useLocalSearchParams } from 'expo-router';
import { File, Paths } from 'expo-file-system';
import { SafeAreaView } from 'react-native-safe-area-context';
import { createBenchmarkReport, reportToCSV, reportToJSON, reportToMarkdown } from '../../shared/phone-benchmarks/index';
import { buildResultsPresentation, buildSweepComparison, type ResultsMetric, type ResultsScope } from '../../shared/phone-benchmarks/presentation';
import { Benchmark, Metrics } from '../src/phoneMetrics';
import { usePhoneResults } from '../src/usePhoneResults';
import { MetricChart, ProcessingChart } from '../src/components/results/ResultsCharts';
import { ResultsMetricDetail } from '../src/components/results/ResultsMetricDetail';
import { Action, Card, SectionTitle, formatDuration, formatNumber, stageLabel, statusLabel, styles } from '../src/components/results/ResultsPrimitives';

const single = (value: string | string[] | undefined) => Array.isArray(value) ? value[0] : value;
const date = (value: number | null | undefined) => value == null ? 'Date unavailable' : new Date(value).toLocaleString(undefined, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' });
const turnName = (index: number | null, id?: string) => index === null ? `Turn ${id?.slice(0, 6) ?? ''}` : `Turn ${index + 1}`;
const conditionLabel = (value: unknown) => Array.isArray(value) && value.length ? value.map(item => {
  try { const parsed: unknown = JSON.parse(String(item)); return Array.isArray(parsed) ? String(parsed[0]) : String(item); } catch { return String(item); }
}).join(', ') : 'unknown';
const emptyMetric = (metric: ResultsMetric): ResultsMetric => ({ ...metric, points: [], rawPoints: [], sampleCount: 0, unavailableCount: 0, operationCount: 0, statusCounts: [], stats: { count: 0, latest: null, min: null, max: null, mean: null }, rawStats: { count: 0, latest: null, min: null, max: null, mean: null } });

export default function MetricsScreen() {
  const params = useLocalSearchParams<{ conversation_id?: string | string[]; sweep_id?: string | string[]; configuration_id?: string | string[] }>();
  const sweepID = single(params.sweep_id);
  const routeConversationID = single(params.conversation_id);
  const routeConfigurationID = single(params.configuration_id);
  const [configurationChoice, setConfigurationChoice] = useState<string | null | undefined>(undefined);
  const configurationID = configurationChoice === undefined ? routeConfigurationID : configurationChoice ?? undefined;
  const results = usePhoneResults({ conversation_id: routeConversationID, sweep_id: sweepID, configuration_id: configurationID });
  const subscribeSync = useCallback((listener: () => void) => results.visible ? Metrics.subscribe(listener) : () => {}, [results.visible]);
  const sync = useSyncExternalStore(subscribeSync, Metrics.getSnapshot, Metrics.getSnapshot);
  const [metricKey, setMetricKey] = useState<string | null>(null);
  const [turnID, setTurnID] = useState<string | null>(null);
  const [attemptID, setAttemptID] = useState<string | null>(null);
  const [historyOpen, setHistoryOpen] = useState(false);
  const [metricsOpen, setMetricsOpen] = useState(false);
  const [capabilitiesOpen, setCapabilitiesOpen] = useState(false);
  const [syncOpen, setSyncOpen] = useState(false);
  const [shareOpen, setShareOpen] = useState(false);
  const [comparisonDetailsOpen, setComparisonDetailsOpen] = useState(false);
  const [metricSearch, setMetricSearch] = useState('');
  const [url, setURL] = useState(sync.url);
  const [error, setError] = useState('');
  const [uploading, setUploading] = useState(false);
  const scroll = useRef<ScrollView>(null);
  useEffect(() => {
    setConfigurationChoice(undefined); setMetricKey(null); setTurnID(null); setAttemptID(null);
  }, [routeConversationID, routeConfigurationID, sweepID]);
  const { projection, snapshot } = results;
  const selectedConversation = projection?.conversations.find(item => item.entity_id === results.selectedId) ?? (!sweepID ? projection?.conversations[0] : undefined);
  const scope: ResultsScope = useMemo(() => ({ ...(sweepID ? { sweep_id: sweepID } : {}), ...(configurationID ? { configuration_id: configurationID } : {}), ...(selectedConversation ? { conversation_id: selectedConversation.entity_id } : {}) }), [sweepID, configurationID, selectedConversation]);
  const presentation = useMemo(() => projection ? buildResultsPresentation(projection, scope) : null, [projection, scope]);
  const narrowed = useMemo(() => !turnID && !attemptID ? presentation : projection ? buildResultsPresentation(projection, { ...scope, ...(turnID ? { turn_id: turnID } : {}), ...(attemptID ? { attempt_id: attemptID } : {}) }) : null, [presentation, projection, scope, turnID, attemptID]);
  const comparison = useMemo(() => projection && sweepID && !selectedConversation && !configurationID ? buildSweepComparison(projection, sweepID, results.plannedSweep ?? undefined) : null, [projection, sweepID, selectedConversation, configurationID, results.plannedSweep]);
  const comparisonMaxima = useMemo(() => {
    const maxima = new Map<string, number>();
    comparison?.configurations.forEach(item => item.stages.forEach(stage => stage.groups.forEach(group => {
      if (group.stats.median !== null) maxima.set(group.key, Math.max(maxima.get(group.key) ?? 0, group.stats.median));
    })));
    return maxima;
  }, [comparison]);
  const metric = presentation?.metrics.find(item => item.key === metricKey);
  const detailMetric = metric && (narrowed?.metrics.find(item => item.key === metric.key) ?? emptyMetric(metric));
  const selectedTurn = presentation?.turns.find(item => item.turn_id === turnID);
  const selectedAttempt = selectedTurn?.segments.find(item => item.attempt_id === attemptID);
  const configuration = projection?.configurations.find(item => item.entity_id === selectedConversation?.context.configuration_id) ?? projection?.configurations[0];
  const current = selectedConversation?.status === 'active' || selectedConversation?.status === 'running';
  const scopeTitle = selectedConversation ? current ? 'Current conversation' : results.selectedId ? 'Saved conversation' : 'Latest conversation' : configurationID ? 'Benchmark configuration' : sweepID ? 'Benchmark sweep' : 'Latest conversation';
  const models = comparison && comparison.configurations.length > 1 ? 'Multiple model configurations' : configuration ? `${configuration.asr?.name ?? 'Speech model unknown'} + ${configuration.reasoning?.name ?? 'No reply model'}` : 'Model details unavailable';
  const memory = presentation?.memory.filter(item => item.stats.count > 0) ?? [];
  const thermal = presentation?.thermal.filter(item => item.sampleCount > 0) ?? [];
  const recordedMetrics = presentation?.metrics.filter(item => !presentation.capabilities.some(capability => capability.name === item.name) || item.rawPoints.some(point => point.value !== null)) ?? [];
  const matchingMetrics = recordedMetrics.filter(item => `${item.label} ${item.name} ${item.source} ${item.scope}`.toLowerCase().includes(metricSearch.toLowerCase()));
  const stats = useMemo(() => syncOpen && results.visible ? Benchmark.store.stats() : { ...Benchmark.store.resultsStatus(), records: 0, bytes: 0 }, [snapshot, sync, results.updatedAt, syncOpen, results.visible]);
  const resetDetail = () => { setMetricKey(null); setTurnID(null); setAttemptID(null); scroll.current?.scrollTo({ y: 0, animated: false }); };
  const openMetric = (key: string, turn?: string, attempt?: string) => { setMetricKey(key); setTurnID(turn ?? null); setAttemptID(attempt ?? null); scroll.current?.scrollTo({ y: 0, animated: false }); };
  const chooseTurn = (id: string | null) => { setTurnID(id); setAttemptID(null); scroll.current?.scrollTo({ y: 0, animated: false }); };
  const chooseConversation = (id: string) => { resetDetail(); if (id) setConfigurationChoice(null); results.selectConversation(id); setHistoryOpen(false); };

  async function flush() {
    setUploading(true); setError('');
    try { await Benchmark.flush(); setError(Benchmark.getError()); }
    catch (cause) { setError(cause instanceof Error ? cause.message : 'Sync failed.'); }
    finally { setUploading(false); }
  }
  async function exportReport(format: 'json' | 'csv' | 'md' | 'summary') {
    if (!snapshot || !presentation) return;
    // Capture the exact displayed selection. Export never reads the store again.
    const displayed = snapshot;
    try {
      const report = createBenchmarkReport(displayed.records, { generated_at_ms: Date.now(), as_of: displayed.as_of, filters: scope, input_already_filtered: true, selected_record_keys: displayed.selected_record_keys });
      if (format === 'summary') {
        const outcomes = comparison?.outcomes ?? presentation.outcomes;
        const content = [`Pheme phone results — ${scopeTitle}`, date(selectedConversation?.started_at_ms ?? projection?.coverage.first_timestamp_ms), models,
          comparison ? `${comparison.outcomes.succeeded ?? 0} / ${comparison.plannedJobs} planned jobs completed` : `${outcomes.succeeded ?? 0} / ${presentation.turns.length} turns completed`,
          `${outcomes.failed ?? 0} failed · ${outcomes.cancelled ?? 0} cancelled · ${outcomes.interrupted ?? 0} interrupted`,
          `${presentation.recordedProcessing.partial ? 'Partial recorded processing' : 'Recorded processing'}: ${formatDuration(presentation.recordedProcessing.duration_ms)} (${presentation.recordedProcessing.attempt_ids.length} operations)`,
          ...memory.map(item => `${item.label}, sampled peak: ${formatNumber(item.stats.max)} ${item.displayUnit} (${item.stats.count} measured readings)`),
          ...presentation.coverage, 'Processing excludes review, playback and idle time. Memory is sampled whole-app memory. Execution completion does not establish incident quality or report saving.',
          `Saved snapshot: ${displayed.as_of}`].join('\n');
        await Share.share({ title: 'Pheme results summary', message: content });
        return;
      }
      const content = format === 'json' ? reportToJSON(report) : format === 'csv' ? reportToCSV(report) : reportToMarkdown(report);
      const file = new File(Paths.cache, `pheme-results-${Date.now()}.${format}`); file.write(content);
      await Share.share({ url: file.uri, title: `Pheme results (${format.toUpperCase()})` });
    } catch (cause) { setError(cause instanceof Error ? cause.message : 'Export failed.'); }
  }
  function removeHistory(id: string) {
    Alert.alert('Remove acknowledged history?', 'This removes the local results for this conversation. Pending, blocked, active and pinned records remain protected.', [
      { text: 'Keep', style: 'cancel' }, { text: 'Remove', style: 'destructive', onPress: () => {
        try { Benchmark.store.removeAcknowledgedHistory(id); if (selectedConversation?.entity_id === id) chooseConversation(''); results.refresh(); }
        catch (cause) { setError(cause instanceof Error ? cause.message : 'History remains protected.'); }
      } },
    ]);
  }
  const togglePin = (id: string) => { try { Benchmark.store.pin(id, !Benchmark.store.isPinned(id)); results.refresh(); } catch (cause) { setError(String(cause)); } };
  const metricTitle = metricKey === 'processing' ? 'Processing time' : metric?.label ?? 'Metric';

  return <SafeAreaView style={styles.safe}><ScrollView ref={scroll} contentContainerStyle={styles.content} keyboardShouldPersistTaps="handled">
    <View style={styles.spread}>{metricKey ? <Action quiet title={`‹ ${turnID ? metricTitle : 'Results'}`} onPress={turnID ? () => chooseTurn(null) : resetDetail} /> : <Link href="/" accessibilityLabel="Back to Chat" style={styles.link}>‹ Chat</Link>}
      {!metricKey && <Link href="/benchmark" style={styles.link}>Benchmarks</Link>}</View>
    {!!metricKey && <Text style={styles.breadcrumb}>Results / {metricTitle}{selectedTurn ? ` / ${turnName(selectedTurn.turn_index, selectedTurn.turn_id)}` : ''}{selectedAttempt ? ` / ${stageLabel(selectedAttempt.stage)}` : ''}</Text>}
    <View style={styles.spread}><Text accessibilityRole="header" style={styles.title}>{metricKey ? selectedTurn ? turnName(selectedTurn.turn_index, selectedTurn.turn_id) : metricTitle : 'Results'}</Text>
      {!metricKey && <Action quiet title="Share" disabled={!snapshot} onPress={() => setShareOpen(!shareOpen)} />}</View>
    <Text style={styles.small}>{metricKey ? `${scopeTitle} · ${date(selectedConversation?.started_at_ms ?? projection?.coverage.first_timestamp_ms)}\n${models}` : `Saved on this phone · ${results.updatedAt ? `Updated ${date(results.updatedAt)}` : 'Waiting for saved results'} · ${stats.pending} pending upload`}</Text>
    {!!(error || results.error) && <Card><Text accessibilityRole="alert" style={styles.error}>{error || results.error}</Text><Action title="Refresh saved results" onPress={results.refresh} /></Card>}

    {!metricKey && <>
      {shareOpen && <Card><SectionTitle title="Share this result" /><Text style={styles.small}>Uses the saved snapshot currently displayed. Audio, transcripts and replies are excluded.</Text><View style={styles.row}><Action title="Share summary" disabled={!snapshot} onPress={() => void exportReport('summary')} />{(['json', 'csv', 'md'] as const).map(format => <Action key={format} title={`Export ${format.toUpperCase()}`} disabled={!snapshot} onPress={() => void exportReport(format)} />)}</View></Card>}
      <Pressable accessibilityRole="button" accessibilityLabel="Choose conversation" accessibilityState={{ expanded: historyOpen }} onPress={() => setHistoryOpen(!historyOpen)}><Card><View style={styles.spread}><Text style={styles.heading}>{scopeTitle}</Text><Text style={styles.link}>{historyOpen ? '⌄' : '›'}</Text></View><Text style={styles.small}>{date(selectedConversation?.started_at_ms ?? projection?.coverage.first_timestamp_ms)} · {models}</Text>{selectedConversation && <Text style={styles.small}>Recorded lifecycle: {statusLabel(selectedConversation.status)}</Text>}</Card></Pressable>
      {historyOpen && <Card><SectionTitle title="Current and saved results" right={`${results.history?.total ?? 0} retained`} />
        {!sweepID && <View style={styles.tabs}>{(['manual', 'sweep', 'acceptance'] as const).map(source => <Pressable key={source} accessibilityRole="button" accessibilityState={{ selected: results.source === source }} onPress={() => { resetDetail(); results.setSource(source); }} style={[styles.tab, results.source === source && styles.activeTab]}><Text style={results.source === source ? styles.link : styles.small}>{source === 'manual' ? 'Manual chats' : source === 'sweep' ? 'Benchmarks' : 'Acceptance'}</Text></Pressable>)}</View>}
        {sweepID && <Action title="All configurations in this sweep" onPress={() => { setConfigurationChoice(null); chooseConversation(''); }} />}
        <TextInput accessibilityLabel="Filter saved results by model or conversation" placeholder="Search model or conversation" value={results.search} onChangeText={results.setSearch} style={styles.input} />
        <View style={styles.tabs}>{(['all', 'today', 'week'] as const).map(period => <Pressable key={period} accessibilityRole="button" accessibilityState={{ selected: results.historyPeriod === period }} onPress={() => results.setHistoryPeriod(period)} style={[styles.tab, results.historyPeriod === period && styles.activeTab]}><Text style={results.historyPeriod === period ? styles.link : styles.small}>{period === 'all' ? 'All dates' : period === 'today' ? 'Today' : 'Past week'}</Text></Pressable>)}</View>
        {(results.history?.items ?? []).map(item => <View key={item.entity_id} style={styles.divider}><Pressable accessibilityRole="button" accessibilityLabel={`Select ${date(item.started_at_ms)} conversation`} onPress={() => chooseConversation(item.entity_id)} style={styles.listRow}><View style={styles.spread}><Text style={styles.value}>{date(item.started_at_ms)}</Text><Text style={styles.pill}>{statusLabel(item.status)}</Text></View><Text style={styles.small}>{item.asr_name ?? 'Unknown speech model'} + {item.reasoning_name ?? 'No reply model'}</Text>{item.entity_id === selectedConversation?.entity_id && <Text style={styles.link}>Selected</Text>}</Pressable><View style={styles.row}><Action quiet title={Benchmark.store.isPinned(item.entity_id) ? 'Unpin' : 'Pin'} onPress={() => togglePin(item.entity_id)} /><Action quiet title="Remove synced history" onPress={() => removeHistory(item.entity_id)} /></View></View>)}
        {!!results.history?.next_cursor && <Action title="Load more saved results" onPress={results.loadMore} />}
        {!results.history?.total && <Text style={styles.small}>No matching saved conversations.</Text>}
      </Card>}
      {sweepID && (configurationID || selectedConversation) && <Action title="Back to sweep overview" onPress={() => { setConfigurationChoice(null); chooseConversation(''); }} />}
      {!presentation && <Card><Text style={styles.heading}>{results.loading ? 'Loading saved results…' : 'Your results will appear here'}</Text><Text style={styles.body}>Start a conversation in Chat or run a benchmark. Results are saved on this phone and can be viewed offline.</Text></Card>}
      {presentation && <>
        <Card><View style={styles.statsGrid}><View style={styles.stat}><Text style={styles.small}>{comparison ? 'Completed jobs' : 'Completed turns'}</Text><Text style={styles.statValue}>{comparison ? comparison.outcomes.succeeded ?? 0 : presentation.outcomes.succeeded ?? 0} / {comparison ? comparison.plannedJobs : presentation.turns.length}</Text><Text style={styles.small}>{(comparison?.outcomes ?? presentation.outcomes).failed ?? 0} failed · {(comparison?.outcomes ?? presentation.outcomes).cancelled ?? 0} cancelled</Text><Text style={styles.small}>{(comparison?.outcomes ?? presentation.outcomes).interrupted ?? 0} interrupted</Text></View><View style={styles.stat}><Text style={styles.small}>{presentation.recordedProcessing.partial ? 'Partial processing subtotal' : 'Recorded processing'}</Text><Text style={styles.statValue}>{formatDuration(presentation.recordedProcessing.duration_ms)}</Text><Text style={styles.small}>{presentation.recordedProcessing.attempt_ids.length} operations</Text>{presentation.typicalProcessingMs !== null && <Text style={styles.small}>Typical complete turn: {formatDuration(presentation.typicalProcessingMs)}</Text>}</View></View>
          {!!memory.length && <View style={styles.divider}><Text style={styles.small}>Sampled peak app memory</Text>{memory.map(item => <Pressable key={item.key} accessibilityRole="button" accessibilityLabel={`Open ${item.label}`} onPress={() => openMetric(item.key)}><Text style={styles.value}>{formatNumber(item.stats.max)} {item.displayUnit} · {item.stats.count} measured readings</Text></Pressable>)}</View>}
        </Card>
        {!!presentation.coverage.length && <Card><Text style={styles.warning}>{presentation.coverage.join('\n')}</Text></Card>}
        {comparison && <Card><SectionTitle title="Model configurations" /><Text style={styles.small}>Median successful operation time · n = complete successful executions. Loading and warmup stay separate.</Text>
          {comparison.limitations.map((line, index) => <Text key={index} style={styles.warning}>{line}</Text>)}
          <Action quiet title={comparisonDetailsOpen ? 'Hide comparison details' : 'Show comparison details'} onPress={() => setComparisonDetailsOpen(!comparisonDetailsOpen)} />
          {comparisonDetailsOpen && comparison.groups.map((group, index) => <View key={group.key} style={styles.divider}><Text style={styles.small}>Population {index + 1}: {stageLabel(group.stage)} · {group.device.hardware} / {group.device.os_version} · {group.mode ?? 'mode unknown'}</Text><Text style={styles.small}>Charging: {conditionLabel(group.conditions.charging)} · Power mode: {conditionLabel(group.conditions.power_mode)} · Thermal: {conditionLabel(group.conditions.thermal)}</Text><Text selectable style={styles.small}>Dataset {group.dataset_revision ?? 'unknown'} · Protocol {group.protocol_revision ?? 'unknown'} · App {group.device.app_version} ({group.device.build})</Text></View>)}
          {comparison.configurations.map(item => <View key={item.key} style={styles.divider}><Text style={styles.heading}>{item.label}</Text><Text style={styles.small}>{item.outcomes.succeeded ?? 0} / {item.plannedJobs} jobs completed · {item.outcomes.failed ?? 0} failed · {item.outcomes.cancelled ?? 0} cancelled · {item.outcomes.interrupted ?? 0} interrupted</Text>
            {item.stages.map(stage => <View key={stage.stage} style={{ gap: 6 }}><Text style={styles.value}>{stage.label}</Text>{stage.groups.map(group => <View key={group.key} style={{ gap: 3 }}><Text style={styles.body}>{formatDuration(group.stats.median)} median · n={group.eligibleCount} / {group.attemptCount} attempts</Text>{group.stats.median !== null && <View accessibilityLabel={`${stage.label}: ${formatDuration(group.stats.median)} median, ${group.eligibleCount} complete executions`} style={{ height: 14, backgroundColor: '#edf1f3', borderRadius: 4 }}><View style={{ height: 14, width: `${comparisonMaxima.get(group.key) ? Math.max(0, group.stats.median / comparisonMaxima.get(group.key)! * 100) : 0}%`, backgroundColor: stage.stage === 'transcription' ? '#7393c6' : '#197563', borderRadius: 4 }} /></View>}<Text style={styles.small}>Population {comparison.groups.findIndex(candidate => candidate.key === group.key) + 1} · {group.timerBoundary ?? 'Unknown timer'} · {group.source} · {group.scope}</Text>{group.stats.p95 !== null && <Text style={styles.small}>p95 {formatDuration(group.stats.p95)}</Text>}</View>)}</View>)}
            {item.sampledPeakMemoryMiB !== null && <Text style={styles.body}>Sampled peak app memory: {formatNumber(item.sampledPeakMemoryMiB)} MiB</Text>}
            {item.limitations.map((line, index) => <Text key={index} style={styles.warning}>{line}</Text>)}
            {comparisonDetailsOpen && <Text selectable style={styles.small}>{JSON.stringify(item.configuration, null, 2)}</Text>}
            {item.configuration_id && <Action title="View configuration results" onPress={() => { setConfigurationChoice(item.configuration_id); chooseConversation(''); }} />}
          </View>)}
        </Card>}
        <Card><Pressable accessibilityRole="button" accessibilityLabel="Open Processing time" onPress={() => openMetric('processing')}><SectionTitle title="Processing by turn" right="seconds  ›" /></Pressable><ProcessingChart turns={presentation.turns} compact onSelect={(turn, segment) => openMetric('processing', turn.turn_id, segment?.attempt_id)} /><Text style={styles.small}>Tap a stage to inspect its operation. Native-call time excludes review, playback and idle time.</Text></Card>
        {memory.map(item => <Pressable key={item.key} accessibilityRole="button" accessibilityLabel={`Open ${item.label}`} onPress={() => openMetric(item.key)}><Card><SectionTitle title={item.label} right={`${formatNumber(item.stats.max)} ${item.displayUnit} peak  ›`} /><MetricChart metric={item} interval={presentation.interval} compact /><Text style={styles.small}>Whole app · {item.stats.count} measured readings{item.unavailableCount ? ` · ${item.unavailableCount} unavailable` : ''} · tap for graph</Text></Card></Pressable>)}
        {thermal.map(item => <Pressable key={item.key} accessibilityRole="button" accessibilityLabel="Open Thermal state" onPress={() => openMetric(item.key)}><Card><SectionTitle title="Thermal state" right="›" /><MetricChart metric={item} interval={presentation.interval} compact /><Text style={styles.small}>{item.sampleCount} readings · categories, not temperature</Text></Card></Pressable>)}
        <Action title={`${metricsOpen ? 'Hide' : 'All'} metrics${metricsOpen ? '' : ` · ${recordedMetrics.length}`}`} accessibilityLabel="All metrics" onPress={() => setMetricsOpen(!metricsOpen)} />
        {metricsOpen && <Card><SectionTitle title="All metrics" /><TextInput accessibilityLabel="Search metrics" placeholder="Search CPU, battery, timings…" value={metricSearch} onChangeText={setMetricSearch} style={styles.input} />
          {matchingMetrics.map(item => <Pressable key={item.key} accessibilityRole="button" accessibilityLabel={`Open ${item.label}`} onPress={() => openMetric(item.key)} style={styles.listRow}><View style={styles.spread}><Text style={styles.value}>{item.label}</Text><Text style={styles.link}>›</Text></View><Text style={styles.small}>{item.stats.count ? `${formatNumber(item.stats.latest)} ${item.displayUnit} latest · ${item.sampleCount} readings` : item.statusCounts.length ? item.statusCounts.map(status => `${status.value}: ${status.count}`).join(' · ') : 'Required measurement unavailable'}{item.unavailableCount ? ` · ${item.unavailableCount} unavailable` : ''}</Text><Text style={styles.small}>{item.scope} · {item.source}</Text></Pressable>)}
          {!matchingMetrics.length && <Text style={styles.small}>{recordedMetrics.length ? 'No metrics match your search.' : 'No measurements retained in this selection.'}</Text>}
        </Card>}
      </>}
      <Action quiet title={`${capabilitiesOpen ? 'Hide' : 'View'} measurement capabilities`} onPress={() => setCapabilitiesOpen(!capabilitiesOpen)} />
      {capabilitiesOpen && <Card><SectionTitle title="Measurement capabilities" /><Text style={styles.body}>CPU and memory describe the whole app process. Thermal state is a category. Battery level does not measure power or energy.</Text>{(presentation?.capabilities ?? []).map(item => <Text key={item.name} style={styles.small}>{item.label}: {item.reasons.join('; ') || 'No provider available.'}</Text>)}<Text style={styles.small}>GPU utilization, watts and joules are unavailable without a measurement provider. Quality and incident-task success are unscored. Completed operations do not establish report correctness, saving or dispatch.</Text><Text style={styles.small}>Resource collection may have been disabled. Missing readings remain gaps; valid zero values remain measurements.</Text></Card>}
      <Action quiet title={`${syncOpen ? 'Hide' : 'Show'} Sync details`} onPress={() => setSyncOpen(!syncOpen)} />
      {syncOpen && <Card><SectionTitle title="Sync details" /><Text style={styles.body}>{stats.pending} observations waiting · {stats.blocked} blocked</Text><Text style={styles.small}>Legacy batches: {sync.pending} waiting / {sync.blocked} blocked. {sync.enabled ? 'Uploads enabled.' : 'Uploads paused.'}</Text><Text style={styles.small}>Last acknowledgement: {stats.lastUpload ? date(stats.lastUpload) : 'Not yet received'} · Retained archive: {stats.records} records / {formatNumber(stats.bytes / 1024)} KiB</Text><TextInput accessibilityLabel="Metrics server address" autoCapitalize="none" autoCorrect={false} value={url} onChangeText={setURL} style={styles.input} placeholder="http://192.168.1.40:8080" /><View style={styles.row}><Action title="Save and enable" onPress={() => { try { Metrics.configure(url, true); setError(''); void flush(); } catch (cause) { setError(String(cause)); } }} /><Action title={sync.enabled ? 'Pause uploads' : 'Resume uploads'} onPress={() => { try { Metrics.configure(url, !sync.enabled); setError(''); } catch (cause) { setError(String(cause)); } }} /><Action title={uploading ? 'Syncing…' : 'Sync now'} disabled={uploading} onPress={() => void flush()} />{!!stats.blocked && <Action title="Retry blocked" onPress={() => { Benchmark.store.retryBlocked(); void flush(); }} />}</View>{!!sync.error && <Text style={styles.error}>{sync.error}</Text>}</Card>}
    </>}
    {metricKey && metricKey !== 'processing' && presentation && metric && detailMetric && <ResultsMetricDetail presentation={presentation} metric={detailMetric} fullMetric={metric} turnId={turnID} attemptId={attemptID} onTurn={chooseTurn} onAttempt={setAttemptID} />}
    {metricKey === 'processing' && presentation && narrowed && <>
      <View style={styles.tabs}><Pressable accessibilityRole="button" accessibilityState={{ selected: !turnID }} style={[styles.tab, !turnID && styles.activeTab]} onPress={() => chooseTurn(null)}><Text style={styles.link}>All turns</Text></Pressable><View style={styles.tab}><Text style={styles.small}>{selectedTurn ? turnName(selectedTurn.turn_index, selectedTurn.turn_id) : 'Choose a turn below'}</Text></View></View>
      {selectedAttempt && <Card><SectionTitle title={stageLabel(selectedAttempt.stage)} right={statusLabel(selectedAttempt.status)} /><Text style={styles.statValue}>{formatDuration(selectedAttempt.duration_ms)}</Text><Text style={styles.small}>{models}</Text><Text style={styles.small}>Native-call duration · {selectedAttempt.completeness} collection</Text></Card>}
      <Card><SectionTitle title={selectedTurn ? turnName(selectedTurn.turn_index, selectedTurn.turn_id) : 'Whole conversation'} right="seconds" /><ProcessingChart turns={narrowed.turns} onSelect={(turn, segment) => { chooseTurn(turn.turn_id); setAttemptID(segment?.attempt_id ?? null); }} /><Text style={styles.body}>{narrowed.recordedProcessing.partial ? 'Partial subtotal' : 'Recorded processing'}: {formatDuration(narrowed.recordedProcessing.duration_ms)}</Text><Text style={styles.small}>{narrowed.recordedProcessing.definition}</Text>{narrowed.recordedProcessing.reasons.map((reason, index) => <Text key={index} style={styles.warning}>{reason}</Text>)}</Card>
      <Card><SectionTitle title={selectedTurn ? 'Operations' : 'Inspect a turn'} />{(selectedTurn ? [selectedTurn] : presentation.turns).map(turn => <View key={turn.turn_id} style={styles.divider}><Action quiet title={`${turnName(turn.turn_index, turn.turn_id)} · ${statusLabel(turn.status)}`} accessibilityLabel={`Inspect ${turnName(turn.turn_index, turn.turn_id)}`} onPress={() => chooseTurn(turn.turn_id)} />{turn.segments.map((segment, index) => <Pressable accessibilityRole="button" key={segment.attempt_id} onPress={() => { chooseTurn(turn.turn_id); setAttemptID(segment.attempt_id); }} style={[styles.listRow, segment.attempt_id === attemptID && styles.selected]}><Text style={styles.value}>{stageLabel(segment.stage)} · attempt {turn.segments.slice(0, index + 1).filter(item => item.stage === segment.stage).length}</Text><Text style={styles.small}>{formatDuration(segment.duration_ms)} · {statusLabel(segment.status)} · {segment.completeness}</Text>{segment.attempt_id === attemptID && <Text selectable style={styles.small}>{segment.attempt_id}{'\n'}{date(segment.started_at_ms)} → {date(segment.finished_at_ms)}</Text>}</Pressable>)}</View>)}</Card>
      {!!turnID && <Action quiet title="Back to all turns" onPress={() => chooseTurn(null)} />}
    </>}
    {!!metricKey && metricKey !== 'processing' && !metric && <Card><Text style={styles.body}>This metric is not available in the current saved snapshot.</Text><Action title="Back to Results" onPress={resetDetail} /></Card>}
  </ScrollView></SafeAreaView>;
}
