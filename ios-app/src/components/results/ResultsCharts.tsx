import { memo, useMemo, useState } from 'react';
import { Pressable, StyleSheet, Text, View } from 'react-native';
import Svg, { Circle, G, Line, Path, Rect, Text as SvgText } from 'react-native-svg';
import type { MetricPoint, ProcessingSegment, ProcessingTurn, ResultsMetric } from '../../../../shared/phone-benchmarks/presentation';
import { formatDuration, formatNumber, stageLabel, statusLabel, styles } from './ResultsPrimitives';

const speech = '#7faed0', reply = '#23816d', ink = '#657187', grid = '#e7edf1';
const finite = (value: unknown): value is number => typeof value === 'number' && Number.isFinite(value);
const stateColors: Record<string, string> = { nominal: '#23816d', fair: '#c6a446', serious: '#c47b36', critical: '#b34d49', unknown: '#8995a5' };
type Interval = { start_ms: number | null; end_ms: number | null };

export const MetricChart = memo(function MetricChart({ metric, interval, compact = false, onPoint }: {
  metric: ResultsMetric; interval?: Interval; compact?: boolean; onPoint?: (point: MetricPoint) => void;
}) {
  const [width, setWidth] = useState(300);
  const [picked, setPicked] = useState<MetricPoint | null>(null);
  const points = metric.points;
  const timestamps = points.map(point => point.timestamp_ms).filter(finite);
  const start = interval?.start_ms ?? (timestamps.length ? Math.min(...timestamps) : 0);
  const end = Math.max(start, interval?.end_ms ?? (timestamps.length ? Math.max(...timestamps) : start));
  const categorical = metric.kind === 'categorical';
  const categories = useMemo(() => metric.name === 'thermal_state'
    ? ['nominal', 'fair', 'serious', 'critical', ...metric.statusCounts.map(item => String(item.value)).filter(value => !['nominal', 'fair', 'serious', 'critical'].includes(value))].slice(0, 8)
    : metric.statusCounts.map(item => String(item.value)).slice(0, 8), [metric.name, metric.statusCounts]);
  const left = categorical ? 65 : 56, right = Math.max(left + 30, width - 12);
  const height = categorical ? Math.max(108, categories.length * 25 + 42) : compact ? 140 : 205;
  const top = 12, bottom = height - 28;
  const x = (timestamp: number) => start === end ? (left + right) / 2 : left + Math.max(0, Math.min(1, (timestamp - start) / (end - start))) * (right - left);
  const numeric = points.filter(point => finite(point.displayValue));
  const minimum = metric.stats.min ?? 0, maximum = metric.stats.max ?? 0;
  const range = maximum - minimum;
  const low = range > 0 ? minimum >= 0 ? Math.max(0, minimum - range * 0.12) : minimum - range * 0.12 : Math.min(0, minimum);
  const high = range > 0 ? maximum + range * 0.12 : maximum > 0 ? maximum * 1.15 : 1;
  const y = (value: number) => bottom - (value - low) / Math.max(1e-9, high - low) * (bottom - top);
  const paths: string[] = [];
  let path = '', prior: MetricPoint | null = null;
  for (const point of points) {
    if (!finite(point.displayValue)) { if (path) paths.push(path); path = ''; prior = null; continue; }
    const move = !prior || point.breakBefore || point.segment !== prior.segment;
    if (move && path) { paths.push(path); path = ''; }
    path += `${move ? 'M' : 'L'}${x(point.timestamp_ms).toFixed(2)},${y(point.displayValue).toFixed(2)} `;
    prior = point;
  }
  if (path) paths.push(path);
  const choose = (point: MetricPoint) => { setPicked(point); onPoint?.(point); };
  const selected = picked && points.find(point => point.timestamp_ms === picked.timestamp_ms && point.segment === picked.segment);
  const label = `${metric.label}. ${metric.sampleCount} readings, ${metric.operationCount} operations. ${metric.stats.count ? `Minimum ${formatNumber(metric.stats.min)}, maximum ${formatNumber(metric.stats.max)} ${metric.displayUnit}.` : metric.statusCounts.map(item => `${item.value}: ${item.count}`).join(', ')} Gaps have no recorded readings.`;

  return <View style={local.chart} onLayout={event => setWidth(Math.max(180, event.nativeEvent.layout.width))}>
    {!metric.stats.count && !metric.statusCounts.length ? <Text style={styles.small}>No measured readings in this selection.</Text> : <>
      <View accessible accessibilityLabel={label} accessibilityRole="image">
        <Svg width="100%" height={height} viewBox={`0 0 ${width} ${height}`}>
          {categorical ? categories.map((category, index) => <Line key={category} x1={left} x2={right} y1={top + 12 + index * 25} y2={top + 12 + index * 25} stroke={grid} />)
            : [0, 0.5, 1].map(ratio => {
              const value = low + (high - low) * ratio;
              return <Line key={ratio} x1={left} x2={right} y1={y(value)} y2={y(value)} stroke={grid} />;
            })}
          {categorical ? categories.map((category, index) => <SvgText key={category} x={left - 7} y={top + 16 + index * 25} textAnchor="end" fill={ink} fontSize={11}>{category}</SvgText>)
            : [0, 0.5, 1].map(ratio => {
              const value = low + (high - low) * ratio;
              return <SvgText key={ratio} x={left - 6} y={y(value) + 4} textAnchor="end" fill={ink} fontSize={10}>{Math.abs(value) >= 100000 ? value.toExponential(1) : Math.abs(value) >= 1000 ? Math.round(value).toLocaleString() : formatNumber(value)}</SvgText>;
            })}
          {categorical ? points.map((point, index) => {
            const row = categories.indexOf(String(point.displayValue));
            if (point.displayValue === null || row < 0) return null;
            const next = points[index + 1];
            const continuing = next && !next.breakBefore && point.segment === next.segment;
            const span = continuing ? Math.max(3, x(next.timestamp_ms) - x(point.timestamp_ms)) : 4;
            return <Rect key={index} x={x(point.timestamp_ms) - (continuing ? 0 : 2)} y={top + 5 + row * 25} width={span} height={14} rx={3}
              fill={stateColors[String(point.displayValue)] ?? reply} onPress={() => choose(point)} />;
          }) : <>
            {paths.map((value, index) => {
              const coordinates = [...value.matchAll(/[ML]([-\d.]+),/g)];
              return coordinates.length > 1 ? <Path key={`fill-${index}`} d={`${value}L${coordinates.at(-1)![1]},${bottom} L${coordinates[0][1]},${bottom} Z`} fill="#e4f3ed" /> : null;
            })}
            {paths.map((value, index) => <Path key={index} d={value} fill="none" stroke={reply} strokeWidth={2.2} strokeLinejoin="round" strokeLinecap="round" />)}
            {numeric.map((point, index) => <G key={index} onPress={() => choose(point)}><Circle cx={x(point.timestamp_ms)} cy={y(point.displayValue as number)} r={numeric.length === 1 ? 5 : 2.5} fill={reply} />{!compact && <Circle cx={x(point.timestamp_ms)} cy={y(point.displayValue as number)} r={12} fill="transparent" />}</G>)}
          </>}
          {[0, 0.5, 1].map(ratio => <SvgText key={ratio} x={left + (right - left) * ratio} y={height - 7} textAnchor={ratio === 0 ? 'start' : ratio === 1 ? 'end' : 'middle'} fill={ink} fontSize={10}>{formatNumber((end - start) * ratio / 1000)} s</SvgText>)}
        </Svg>
      </View>
      <View style={styles.spread}><Text style={styles.small}>{categorical ? 'Recorded states' : metric.displayUnit} · elapsed time</Text><Text style={styles.small}>{metric.stats.count === 1 || metric.statusCounts.reduce((sum, item) => sum + item.count, 0) === 1 ? 'Single reading' : 'Gaps preserved'}</Text></View>
      {categorical && <Text style={styles.small}>{metric.statusCounts.map(item => `${String(item.value)}: ${item.count}`).join(' · ')}</Text>}
      {selected && <Text accessibilityLiveRegion="polite" style={styles.small}>{String(selected.displayValue)} {metric.displayUnit} at +{formatNumber((selected.timestamp_ms - start) / 1000)} s · {stageLabel(selected.stage)} · {statusLabel(selected.outcome)}</Text>}
      {metric.downsampled && <Text style={styles.small}>{points.length} points shown from {metric.sampleCount} readings. Statistics and exports retain every reading.{metric.omittedSegments ? ` ${metric.omittedSegments} operation segments omitted from the compact plot.` : ''}</Text>}
    </>}
  </View>;
});

export const ProcessingChart = memo(function ProcessingChart({ turns, onSelect, compact = false }: {
  turns: ProcessingTurn[]; onSelect: (turn: ProcessingTurn, segment?: ProcessingSegment) => void; compact?: boolean;
}) {
  const [expanded, setExpanded] = useState(false);
  const max = turns.reduce((value, turn) => Math.max(value, turn.recordedProcessing.duration_ms ?? 0), 1);
  const visible = compact && !expanded ? turns.slice(0, 12) : turns;
  return <View style={local.chart}>
    <View style={styles.row}><View style={[local.legend, { backgroundColor: speech }]} /><Text style={styles.small}>Speech processing</Text><View style={[local.legend, { backgroundColor: reply }]} /><Text style={styles.small}>Reply generation</Text></View>
    {!turns.length && <Text style={styles.small}>No turn processing recorded in this selection.</Text>}
    {visible.map(turn => {
      const name = turn.turn_index === null ? 'Turn' : `Turn ${turn.turn_index + 1}`;
      return <View key={`${turn.installation_id}:${turn.turn_id}`} style={local.turn}>
        <Pressable accessibilityRole="button" accessibilityLabel={`Inspect ${name}`} onPress={() => onSelect(turn)} style={styles.spread}>
          <Text style={styles.value}>{name}</Text><Text style={styles.small}>{formatDuration(turn.recordedProcessing.duration_ms)}{turn.recordedProcessing.partial ? ' · partial' : ''} · {statusLabel(turn.status)}</Text>
        </Pressable>
        <View style={local.track}>
          {turn.segments.filter(segment => segment.duration_ms !== null && segment.duration_ms > 0).map(segment => <Pressable key={segment.attempt_id}
            accessibilityRole="button" accessibilityLabel={`${name}, ${stageLabel(segment.stage)}, ${formatDuration(segment.duration_ms)}, ${statusLabel(segment.status)}`}
            onPress={() => onSelect(turn, segment)} style={{ width: `${Math.min(100, (segment.duration_ms ?? 0) / max * 100)}%`, minHeight: 30,
              backgroundColor: segment.stage === 'transcription' ? speech : reply, opacity: segment.status === 'succeeded' ? 1 : 0.65,
              borderWidth: segment.status === 'succeeded' ? 0 : 1, borderColor: '#694f3e', borderStyle: segment.status === 'succeeded' ? 'solid' : 'dashed' }} />)}
        </View>
        <View style={styles.row}>{turn.segments.map((segment, index) => ({ segment, number: turn.segments.slice(0, index + 1).filter(item => item.stage === segment.stage).length }))
          .filter(({ segment }) => !compact || segment.duration_ms === null || segment.duration_ms === 0 || segment.status !== 'succeeded')
          .map(({ segment, number }) => <Pressable key={segment.attempt_id} accessibilityRole="button"
          accessibilityLabel={`${name}, ${stageLabel(segment.stage)}, attempt ${number}, ${formatDuration(segment.duration_ms)}, ${statusLabel(segment.status)}`}
          onPress={() => onSelect(turn, segment)} style={local.segmentLabel}>
          <Text style={styles.small}>{stageLabel(segment.stage)}{turn.segments.filter(item => item.stage === segment.stage).length > 1 ? ` #${number}` : ''} {formatDuration(segment.duration_ms)}{segment.status !== 'succeeded' ? ` · ${statusLabel(segment.status)}` : ''}</Text>
        </Pressable>)}</View>
        {!turn.segments.length && <Text style={styles.small}>No recorded operation durations.</Text>}
      </View>;
    })}
    {!!turns.length && <View style={styles.spread}><Text style={styles.small}>0 s</Text><Text style={styles.small}>{formatDuration(max)}</Text></View>}
    {compact && turns.length > 12 && !expanded && <Pressable accessibilityRole="button" onPress={() => setExpanded(true)} style={styles.button}><Text style={styles.link}>Show all {turns.length} turns</Text></Pressable>}
  </View>;
});

const local = StyleSheet.create({ chart: { gap: 8, width: '100%' }, legend: { width: 9, height: 9, borderRadius: 2 }, turn: { gap: 3 },
  track: { flexDirection: 'row', minHeight: 30, backgroundColor: '#f0f3f5', borderRadius: 5, overflow: 'hidden' },
  segmentLabel: { minHeight: 44, justifyContent: 'center' } });
