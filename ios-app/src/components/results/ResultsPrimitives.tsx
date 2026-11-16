import type { ReactNode } from 'react';
import { Pressable, StyleSheet, Text, View, type StyleProp, type ViewStyle } from 'react-native';
import type { ResultsMetric } from '../../../../shared/phone-benchmarks/presentation';

export const formatNumber = (value: number | null) => value === null ? 'Unavailable' : Number(value.toFixed(2)).toLocaleString();
export const formatDuration = (ms: number | null) => ms === null ? 'Unavailable' : `${formatNumber(ms / 1000)} s`;
export const stageLabel = (stage: string | null) => stage === 'transcription' ? 'Speech processing' : stage === 'reasoning' ? 'Reply generation' : stage === 'loading' ? 'Model loading' : stage === 'warmup' ? 'Warmup' : stage ?? 'Unknown operation';
export const statusLabel = (status: string | null) => status === 'succeeded' ? 'Completed' : status ? status.charAt(0).toUpperCase() + status.slice(1) : 'Unknown outcome';

export function Action({ title, onPress, disabled = false, quiet = false, accessibilityLabel }: { title: string; onPress: () => void; disabled?: boolean; quiet?: boolean; accessibilityLabel?: string }) {
  return <Pressable accessibilityRole="button" accessibilityLabel={accessibilityLabel ?? title} accessibilityState={{ disabled }} disabled={disabled} onPress={onPress} style={[styles.button, quiet && styles.quietButton, disabled && styles.disabled]}><Text style={styles.link}>{title}</Text></Pressable>;
}
export function Card({ children, style }: { children: ReactNode; style?: StyleProp<ViewStyle> }) {
  return <View style={[styles.card, style]}>{children}</View>;
}
export function SectionTitle({ title, right }: { title: string; right?: string }) {
  return <View style={styles.spread}><Text style={styles.heading}>{title}</Text>{!!right && <Text style={styles.small}>{right}</Text>}</View>;
}
export function MetricStats({ metric }: { metric: ResultsMetric }) {
  if (!metric.stats.count) return null;
  return <Card><View style={styles.statsGrid}>{([
    ['Latest', metric.stats.latest], [metric.name.includes('memory') || metric.name.includes('footprint') ? 'Sampled peak' : 'Maximum', metric.stats.max],
    ['Minimum', metric.stats.min], ['Average', metric.stats.mean],
  ] as const).map(([label, value]) => <View key={label} style={styles.stat}><Text style={styles.small}>{label}</Text><Text style={styles.statValue}>{formatNumber(value)} <Text style={styles.unit}>{metric.displayUnit}</Text></Text></View>)}</View></Card>;
}
export const styles = StyleSheet.create({
  safe: { flex: 1, backgroundColor: '#f5f7f9' }, content: { padding: 20, gap: 14, paddingBottom: 44, width: '100%', maxWidth: 820, alignSelf: 'center' },
  title: { fontSize: 30, lineHeight: 37, fontWeight: '700', letterSpacing: -0.7, color: '#182132', flexShrink: 1 },
  heading: { fontSize: 17, lineHeight: 24, fontWeight: '600', color: '#182132', flexShrink: 1 },
  body: { fontSize: 14, lineHeight: 21, color: '#354253', flexShrink: 1 },
  small: { fontSize: 12, lineHeight: 18, color: '#657187', flexShrink: 1 },
  value: { fontSize: 15, lineHeight: 22, fontWeight: '600', color: '#182132' },
  statValue: { fontSize: 26, lineHeight: 34, fontWeight: '700', color: '#182132', letterSpacing: -0.5 },
  unit: { fontSize: 14, fontWeight: '500', letterSpacing: 0 },
  link: { fontSize: 14, lineHeight: 21, color: '#197563', fontWeight: '600', flexShrink: 1 },
  card: { borderWidth: 1, borderColor: '#e1e6eb', borderRadius: 14, padding: 16, backgroundColor: '#fff', gap: 10 },
  row: { flexDirection: 'row', flexWrap: 'wrap', alignItems: 'center', gap: 8 },
  spread: { flexDirection: 'row', flexWrap: 'wrap', justifyContent: 'space-between', alignItems: 'center', gap: 8 },
  button: { backgroundColor: '#eaf6f2', borderRadius: 9, paddingHorizontal: 12, paddingVertical: 11, minHeight: 44, justifyContent: 'center' },
  quietButton: { backgroundColor: 'transparent', paddingHorizontal: 0 }, disabled: { opacity: 0.4 },
  input: { borderWidth: 1, borderColor: '#d9e0e7', borderRadius: 9, padding: 12, minHeight: 44, color: '#182132', fontSize: 14, backgroundColor: '#fff' },
  error: { color: '#a33727', fontSize: 13, lineHeight: 20 }, warning: { color: '#8c611c', fontSize: 12, lineHeight: 18 },
  divider: { borderTopWidth: 1, borderColor: '#e7ebef', paddingTop: 12, gap: 7 },
  selected: { backgroundColor: '#ebf7f3', borderColor: '#23816d' },
  listRow: { minHeight: 48, paddingVertical: 10, gap: 3, borderBottomWidth: 1, borderBottomColor: '#e9edf1' },
  statsGrid: { flexDirection: 'row', flexWrap: 'wrap', rowGap: 16 }, stat: { width: '50%', minWidth: 120, paddingRight: 8, gap: 2 },
  pill: { color: '#197563', backgroundColor: '#eaf6f2', borderRadius: 15, overflow: 'hidden', paddingHorizontal: 10, paddingVertical: 4, fontSize: 12, fontWeight: '600' },
  tabs: { flexDirection: 'row', flexWrap: 'wrap', borderRadius: 10, padding: 4, backgroundColor: '#e9edf1', gap: 4 },
  tab: { minHeight: 44, flexGrow: 1, paddingHorizontal: 10, paddingVertical: 11, alignItems: 'center', justifyContent: 'center', borderRadius: 8 },
  activeTab: { backgroundColor: '#fff' }, breadcrumb: { fontSize: 12, lineHeight: 18, color: '#197563' },
});
