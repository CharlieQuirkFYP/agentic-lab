import { useEffect, useMemo, useState } from 'react';
import { Pressable, Text, View } from 'react-native';
import type { MetricPoint, ResultsMetric, ResultsPresentation } from '../../../../shared/phone-benchmarks/presentation';
import { MetricChart } from './ResultsCharts';
import { Action, Card, MetricStats, SectionTitle, formatDuration, formatNumber, stageLabel, statusLabel, styles } from './ResultsPrimitives';

interface Props {
  presentation: ResultsPresentation;
  metric: ResultsMetric;
  fullMetric: ResultsMetric;
  turnId: string | null;
  attemptId: string | null;
  onTurn: (id: string | null) => void;
  onAttempt: (id: string | null) => void;
}
const timestamp = (value: number) => new Date(value).toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit', second: '2-digit' });
const turnLabel = (index: number | null) => index === null ? 'Unnumbered turn' : `Turn ${index + 1}`;
const pointValue = (point: MetricPoint, metric: ResultsMetric) => point.displayValue === null
  ? `Unavailable${point.unavailable_reason ? `: ${point.unavailable_reason}` : ''}`
  : `${typeof point.displayValue === 'number' ? formatNumber(point.displayValue) : String(point.displayValue)}${metric.kind === 'categorical' ? '' : ` ${metric.displayUnit}`}`;
const object = (value: unknown): Record<string, unknown> => value !== null && typeof value === 'object' ? value as Record<string, unknown> : {};

export function ResultsMetricDetail({ presentation, metric, fullMetric, turnId, attemptId, onTurn, onAttempt }: Props) {
  const [readingsOpen, setReadingsOpen] = useState(false);
  const [readingLimit, setReadingLimit] = useState(30);
  const [selectedPoint, setSelectedPoint] = useState<MetricPoint | null>(null);
  const [choosingTurn, setChoosingTurn] = useState(false);
  const turn = presentation.turns.find(item => item.turn_id === turnId);
  const attempt = turn?.segments.find(item => item.attempt_id === attemptId);
  const measuredCount = metric.stats.count + metric.statusCounts.reduce((sum, item) => sum + item.count, 0);
  useEffect(() => { setSelectedPoint(null); setReadingLimit(30); }, [metric.key, turnId, attemptId]);
  const interval = useMemo(() => {
    if (!turn) return presentation.interval;
    const operations = attempt ? [attempt] : turn.segments;
    const starts = operations.flatMap(item => item.started_at_ms === null ? [] : [item.started_at_ms]);
    const finishes = operations.flatMap(item => item.finished_at_ms === null ? [] : [item.finished_at_ms]);
    const firstPoint = metric.rawPoints[0]?.timestamp_ms ?? null;
    const lastPoint = metric.rawPoints.at(-1)?.timestamp_ms ?? null;
    return { start_ms: starts.length ? starts.reduce((min, value) => Math.min(min, value)) : firstPoint,
      end_ms: finishes.length ? finishes.reduce((max, value) => Math.max(max, value), lastPoint ?? finishes[0]) : lastPoint };
  }, [presentation.interval, turn, attempt, metric.rawPoints]);
  const context = (point: MetricPoint) => {
    const pointTurn = presentation.turns.find(item => item.turn_id === point.context.turn_id);
    const pointAttempt = pointTurn?.segments.find(item => item.attempt_id === point.context.attempt_id);
    const retry = pointTurn && pointAttempt ? pointTurn.segments.filter(item => item.stage === pointAttempt.stage).findIndex(item => item.attempt_id === pointAttempt.attempt_id) + 1 : null;
    return `${pointTurn ? turnLabel(pointTurn.turn_index) : 'Turn not recorded'} · ${stageLabel(point.stage)}${retry ? ` · attempt ${retry}` : ''} · ${statusLabel(point.outcome)}`;
  };
  const elapsed = (point: MetricPoint) => (point.timestamp_ms - (interval.start_ms ?? point.timestamp_ms)) / 1000;
  const model = attempt ? object(fullMetric.configuration?.[attempt.stage === 'transcription' ? 'asr' : 'reasoning']).name : null;
  const attemptNumber = attempt && turn ? turn.segments.filter(item => item.stage === attempt.stage).findIndex(item => item.attempt_id === attempt.attempt_id) + 1 : null;
  const memory = metric.name === 'resident_memory_bytes' || metric.name === 'physical_footprint_bytes';

  return <>
    <View style={styles.tabs}>
      <Pressable accessibilityRole="button" accessibilityState={{ selected: !turnId }} style={[styles.tab, !turnId && styles.activeTab]} onPress={() => { onTurn(null); setChoosingTurn(false); }}><Text style={!turnId ? styles.link : styles.small}>All turns</Text></Pressable>
      <Pressable accessibilityRole="button" accessibilityLabel="Choose a turn" accessibilityState={{ expanded: choosingTurn }} style={[styles.tab, !!turnId && styles.activeTab]} onPress={() => setChoosingTurn(!choosingTurn)}><Text style={turnId ? styles.link : styles.small}>{turn ? turnLabel(turn.turn_index) : 'Choose a turn'}</Text></Pressable>
    </View>
    {!!turn && <View style={styles.tabs}>{turn.segments.map((segment, index) => {
      const number = turn.segments.slice(0, index + 1).filter(item => item.stage === segment.stage).length;
      return <Pressable key={segment.attempt_id} accessibilityRole="button" accessibilityLabel={`${stageLabel(segment.stage)}, attempt ${number}`} accessibilityState={{ selected: attemptId === segment.attempt_id }} onPress={() => onAttempt(attemptId === segment.attempt_id ? null : segment.attempt_id)} style={[styles.tab, attemptId === segment.attempt_id && styles.activeTab]}><Text style={attemptId === segment.attempt_id ? styles.link : styles.small}>{stageLabel(segment.stage)}{number > 1 ? ` · retry ${number - 1}` : ''}</Text></Pressable>;
    })}</View>}
    {attempt && <Card><SectionTitle title={stageLabel(attempt.stage)} right={statusLabel(attempt.status)} /><View style={styles.spread}><Text style={styles.statValue}>{formatDuration(attempt.duration_ms)}</Text><Text style={styles.small}>{typeof model === 'string' ? `${model} · ` : ''}attempt {attemptNumber}</Text></View><Text style={styles.small}>Recorded native-call duration · {attempt.completeness} collection</Text></Card>}
    <Card><SectionTitle title={turn ? metric.label : presentation.scope.sweep_id && !presentation.scope.conversation_id ? 'Selected benchmark result' : 'Whole conversation'} right={metric.displayUnit} />
      <MetricChart metric={metric} interval={interval} onPoint={setSelectedPoint} />
      <Text style={styles.small}>{measuredCount} measured readings{metric.unavailableCount ? ` · ${metric.unavailableCount} unavailable` : ''}{attempt ? ' during this operation' : turn ? ' during this turn' : ` across ${metric.operationCount} operations`}</Text>
      {metric.downsampled && <Text style={styles.small}>The graph shows {metric.points.length} selected points. Statistics and readings below use every saved reading.</Text>}
      {!!metric.omittedSegments && <Text style={styles.warning}>{metric.omittedSegments} operation segments omitted from the compact graph. All readings remain available below.</Text>}
      <Text style={styles.small}>Gaps mean no recorded readings. Elapsed time starts at {attempt ? 'this operation' : turn ? 'this turn’s first recorded operation' : 'this result'}.</Text>
    </Card>
    {selectedPoint && <Card><SectionTitle title="Selected reading" right={timestamp(selectedPoint.timestamp_ms)} /><Text style={styles.statValue}>{pointValue(selectedPoint, metric)}</Text><Text style={styles.body}>{context(selectedPoint)}</Text><Text style={styles.small}>{formatNumber(elapsed(selectedPoint))} s elapsed</Text><Action quiet title="Close selected reading" onPress={() => setSelectedPoint(null)} /></Card>}
    <MetricStats metric={metric} />
    {!!metric.statusCounts.length && <Card><SectionTitle title="Recorded states" />{metric.statusCounts.map(item => <View key={String(item.value)} style={styles.spread}><Text style={styles.body}>{String(item.value)}</Text><Text style={styles.small}>{item.count} readings</Text></View>)}</Card>}
    <Text style={styles.small}>{metric.description}</Text>
    {memory && <Text style={styles.small}>Memory belongs to the whole app{attempt ? ', sampled while this operation ran' : ''}. The peak is the highest sampled reading; these readings do not measure model-only allocation.</Text>}
    {(choosingTurn || !turnId) && <Card><SectionTitle title="Inspect a turn" />{presentation.turns.map(item => <Pressable key={item.turn_id} accessibilityRole="button" accessibilityLabel={`Inspect ${turnLabel(item.turn_index)}`} accessibilityState={{ selected: turnId === item.turn_id }} onPress={() => { onTurn(item.turn_id); setChoosingTurn(false); }} style={[styles.listRow, item.turn_id === turnId && styles.selected]}><View style={styles.spread}><Text style={styles.value}>{turnLabel(item.turn_index)}</Text><Text style={styles.link}>{item.segments.length} operations  ›</Text></View><Text style={styles.small}>{statusLabel(item.status)} · {formatDuration(item.recordedProcessing.duration_ms)}{item.recordedProcessing.partial ? ' partial subtotal' : ' processing'}</Text></Pressable>)}{!presentation.turns.length && <Text style={styles.small}>No turn identities were retained for these measurements.</Text>}</Card>}
    <Action quiet title={readingsOpen ? 'Hide readings and measurement details' : 'View readings and measurement details'} accessibilityLabel="View readings and measurement details" onPress={() => setReadingsOpen(!readingsOpen)} />
    {readingsOpen && <Card><SectionTitle title="Measurement details" /><Text selectable style={styles.small}>{metric.name} · {metric.source} · {metric.scope}{'\n'}Stored unit: {metric.unit} · Display unit: {metric.displayUnit}{'\n'}Device: {metric.device.hardware} · {metric.device.os_version}{'\n'}App: {metric.device.app_version} ({metric.device.build})</Text>
      <Text style={styles.small}>Latest, minimum, maximum and average are sample statistics. They are not weighted by time. Raw resource readings do not have an execution percentile.</Text>
      <Text selectable style={styles.small}>Installation: {metric.installation_id}{'\n'}Configuration: {metric.configuration_id ?? 'unavailable'}{'\n'}Dataset: {metric.dataset_revision ?? 'unavailable'}{'\n'}Protocol: {metric.protocol_revision ?? 'unavailable'}</Text>
      {!!metric.configuration && <Text selectable style={styles.small}>{JSON.stringify(metric.configuration, null, 2)}</Text>}
      <SectionTitle title="Saved readings" right={`${metric.rawPoints.length} total`} />
      <Text style={styles.small}>Time / elapsed · value · turn / operation / outcome</Text>
      {metric.rawPoints.slice(0, readingLimit).map((point, index) => <View key={`${point.timestamp_ms}-${index}`} accessible accessibilityLabel={`${timestamp(point.timestamp_ms)}, ${formatNumber(elapsed(point))} seconds elapsed, ${pointValue(point, metric)}, ${context(point)}`} style={styles.divider}><View style={styles.spread}><Text style={styles.small}>{timestamp(point.timestamp_ms)} / {formatNumber(elapsed(point))} s</Text><Text style={styles.value}>{pointValue(point, metric)}</Text></View><Text style={styles.small}>{context(point)}</Text><Text selectable style={styles.small}>Raw: {String(point.value)} {metric.unit}{'\n'}{point.references.map(reference => `${reference.record_id} · event ${reference.event_index} · run ${reference.run_id} · sequence ${reference.sequence}`).join('\n')}</Text></View>)}
      {readingLimit < metric.rawPoints.length && <Action title={`Show more readings (${metric.rawPoints.length - readingLimit} remaining)`} onPress={() => setReadingLimit(value => value + 30)} />}
      {!metric.rawPoints.length && <Text style={styles.body}>No readings were saved for this metric in the selected operation.</Text>}
    </Card>}
    {!!turnId && <Action quiet title="Back to all turns" onPress={() => { onTurn(null); setChoosingTurn(false); }} />}
  </>;
}
