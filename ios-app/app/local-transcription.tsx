import { useEffect, useRef, useState } from 'react';
import { ActivityIndicator, AppState, Pressable, ScrollView, StyleSheet, Text, TextInput, View } from 'react-native';
import { Link } from 'expo-router';
import { SafeAreaView } from 'react-native-safe-area-context';
import * as DocumentPicker from 'expo-document-picker';
import { requestRecordingPermissionsAsync, setAudioModeAsync, useAudioStream, type AudioStreamBuffer } from 'expo-audio';
import Pheme from '../modules/pheme-native/src/PhemeNativeModule';
import { sameSettings } from '../modules/pheme-native/src/client';
import type { Inspection, LocalResult, Model, Observation, Settings } from '../modules/pheme-native/src/PhemeNative.types';
import { latestLocalTranscript, saveLocalTranscript, updateLocalDraft } from '../src/localTranscriptStore';
import { transcribeLocally } from '../src/localWorkflow';
import { readPendingRecording, removePendingRecording, saveRecording } from '../src/recordingStore';
import { pcm16ToWav } from '../src/wav';

const DEFAULTS: Settings = { modelID: '', language: 'en', dictionary: [], threads: 4, maxAudioSeconds: 120, metricsEnabled: true };
const TINY_CHECKSUM = '921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f';

export default function LocalTranscriptionScreen() {
  const [inspection, setInspection] = useState<Inspection | null>(null);
  const [models, setModels] = useState<Model[]>([]);
  const [settings, setSettings] = useState<Settings>(DEFAULTS);
  const [checksum, setChecksum] = useState(TINY_CHECKSUM);
  const [modelName, setModelName] = useState('Whisper tiny.en test model');
  const [message, setMessage] = useState('Import and load a local model to begin.');
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const [recording, setRecording] = useState(false);
  const [seconds, setSeconds] = useState(0);
  const [result, setResult] = useState<LocalResult | null>(null);
  const [draft, setDraft] = useState('');
  const [observation, setObservation] = useState<Observation | null>(null);
  const [batches, setBatches] = useState<string[]>([]);
  const [setupAttempt, setSetupAttempt] = useState(0);
  const mounted = useRef(true);
  const recordingRef = useRef(false);
  const busyRef = useRef(false);
  const chunks = useRef<ArrayBuffer[]>([]);
  const bytes = useRef(0);
  const format = useRef({ sampleRate: 16000, channels: 1 });
  const limit = useRef(120);

  const audio = useAudioStream({ sampleRate: 16000, channels: 1, encoding: 'int16', onBuffer: (buffer: AudioStreamBuffer) => {
    if (!recordingRef.current) return;
    const frameBytes = buffer.channels * 2;
    const maximum = Math.floor(limit.current * buffer.sampleRate) * frameBytes;
    const take = Math.floor(Math.max(0, Math.min(buffer.data.byteLength, maximum - bytes.current)) / frameBytes) * frameBytes;
    if (take) { chunks.current.push(new Uint8Array(buffer.data).slice(0, take).buffer); bytes.current += take; }
    format.current = { sampleRate: buffer.sampleRate, channels: buffer.channels };
  } });

  useEffect(() => {
    mounted.current = true;
    let stopped = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const restore = latestLocalTranscript();
    if (restore) { setResult(restore.result); setDraft(restore.draft); }
    const poll = async () => {
      if (stopped) return;
      try {
        const update = await Pheme.poll();
        if (!stopped) {
          setObservation(update);
          if (update.batchesJSON !== '[]') setBatches(current => {
            const next = [...current, update.batchesJSON];
            while (next.length > 20 || next.reduce((n, value) => n + value.length * 2, 0) > 4 * 1024 * 1024) next.shift();
            return next;
          });
        }
      } catch (e) { if (!stopped) setError(e instanceof Error ? e.message : 'Local observation failed.'); }
      if (!stopped) timer = setTimeout(() => void poll(), 250);
    };
    if (Pheme.available) void (async () => {
      try {
        await Pheme.close();
        if (stopped) return;
        const state = await Pheme.open();
        const imported = await Pheme.listModels();
        const saved = await Pheme.savedSettings();
        if (stopped) return;
        setInspection(state); setModels(imported);
        if (saved && imported.some(item => item.id === saved.modelID)) setSettings(saved);
        else if (imported[0]) setSettings(current => ({ ...current, modelID: imported[0].id }));
        void poll();
      } catch (e) { if (!stopped) setError(e instanceof Error ? e.message : 'Could not open native Pheme.'); }
    })();
    return () => {
      stopped = true; mounted.current = false; recordingRef.current = false;
      if (timer) clearTimeout(timer);
      try { audio.stream.stop(); } catch { /* Already stopped. */ }
      void setAudioModeAsync({ allowsRecording: false, playsInSilentMode: true }).catch(() => {});
      void Pheme.close();
    };
  }, [setupAttempt]);

  useEffect(() => {
    const subscription = AppState.addEventListener('change', state => {
      if (state === 'active') return;
      if (recordingRef.current) {
        recordingRef.current = false; chunks.current = []; bytes.current = 0;
        try { audio.stream.stop(); } catch { /* Already stopped. */ }
        setRecording(false); setMessage('Recording interrupted. Record again when the app is active.');
      }
      void Pheme.cancel().catch(() => {});
    });
    return () => subscription.remove();
  }, [audio.stream]);

  useEffect(() => {
    if (!recording) return;
    const started = Date.now();
    const clock = setInterval(() => setSeconds(Math.floor((Date.now() - started) / 1000)), 250);
    const stop = setTimeout(() => void stopRecording(), limit.current * 1000);
    return () => { clearInterval(clock); clearTimeout(stop); };
  }, [recording]);

  async function operation(work: () => Promise<void>) {
    if (busyRef.current || recordingRef.current) return;
    busyRef.current = true; setBusy(true); setError('');
    try { await work(); }
    catch (e) { if (mounted.current) setError(e instanceof Error ? e.message : 'Local operation failed.'); }
    finally {
      busyRef.current = false;
      if (mounted.current) {
        setBusy(false);
        try { const next = await Pheme.inspect(); if (mounted.current) setInspection(next); } catch { /* Original error is shown. */ }
      }
    }
  }

  async function importModel() {
    const picked = await DocumentPicker.getDocumentAsync({ type: '*/*', copyToCacheDirectory: true });
    if (picked.canceled) return;
    await operation(async () => {
      setMessage('Verifying and importing the selected model…');
      const model = await Pheme.importModel(picked.assets[0].uri, modelName, checksum.trim());
      const imported = await Pheme.listModels();
      if (mounted.current) { setModels(imported); setSettings(current => ({ ...current, modelID: model.id })); setMessage('Model imported. Tap Load model.'); }
    });
  }

  async function transcribe(uri: string, recoveryRecording: boolean) {
    setMessage('Transcribing on this iPhone…');
    const completed = await transcribeLocally(uri, {
      transcribeFile: Pheme.transcribeFile, persist: saveLocalTranscript,
      releaseRecording: () => { if (recoveryRecording && readPendingRecording('local')?.uri === uri) removePendingRecording('local'); },
    });
    if (mounted.current) { setResult(completed); setDraft(completed.transcription.result.text); setMessage('Local transcript saved. Review or edit it below.'); }
  }

  async function startRecording() {
    if (busyRef.current || recordingRef.current || !inspection?.snapshot.ready) return;
    await operation(async () => {
      const permission = await requestRecordingPermissionsAsync();
      if (!mounted.current || AppState.currentState !== 'active') return;
      if (!permission.granted) throw new Error('Microphone permission is denied. Enable it in Settings.');
      await setAudioModeAsync({ allowsRecording: true, playsInSilentMode: true });
      chunks.current = []; bytes.current = 0; limit.current = inspection?.settings?.maxAudioSeconds ?? 120;
      await audio.stream.start();
      if (!mounted.current || AppState.currentState !== 'active') {
        audio.stream.stop();
        await setAudioModeAsync({ allowsRecording: false, playsInSilentMode: true });
        return;
      }
      recordingRef.current = true; setRecording(true); setSeconds(0); setMessage('Recording locally. Tap Stop to transcribe.');
    });
  }

  async function stopRecording() {
    if (!recordingRef.current) return;
    recordingRef.current = false; setRecording(false);
    try { audio.stream.stop(); } catch { /* Stop is idempotent. */ }
    await operation(async () => {
      await setAudioModeAsync({ allowsRecording: false, playsInSilentMode: true });
      if (!chunks.current.length) throw new Error('No microphone audio was captured.');
      const current = format.current;
      const wav = pcm16ToWav(chunks.current, current.sampleRate, current.channels);
      const duration = bytes.current / (current.sampleRate * current.channels * 2);
      const saved = saveRecording(wav, duration, 'local'); chunks.current = []; bytes.current = 0;
      await transcribe(saved.uri, true);
    });
  }

  const ready = Boolean(inspection?.snapshot.ready && !inspection.closed && sameSettings(settings, inspection.settings));
  const pending = readPendingRecording('local');
  const disabled = busy || recording || !Pheme.available;
  return <SafeAreaView style={styles.safe}><ScrollView contentContainerStyle={styles.content} keyboardShouldPersistTaps="handled">
    <Link href="/" style={styles.link}>← Remote development chat</Link>
    <Text style={styles.title}>On-device transcription</Text>
    <Text style={styles.body}>Pheme runs inside this app. Laptop services are not used for transcription. Local reply generation and benchmark uploads are later milestones.</Text>
    {!Pheme.available && <Text style={styles.error}>Install the custom iOS development build to use Pheme. This feature is unavailable in Expo Go.</Text>}
    {!!error && <Text accessibilityRole="alert" style={styles.error}>{error}</Text>}
    {!!error && Pheme.available && <Action title="Reopen local setup" disabled={busy || recording} onPress={() => { setError(''); setSetupAttempt(current => current + 1); }} />}
    <Text style={styles.status}>{busy && <ActivityIndicator />} {message}</Text>
    <View style={styles.card}><Text style={styles.heading}>Local model</Text>
      <TextInput accessibilityLabel="Model name" style={styles.input} value={modelName} onChangeText={setModelName} editable={!disabled} />
      <TextInput accessibilityLabel="Trusted SHA-256" style={styles.input} value={checksum} onChangeText={setChecksum} autoCapitalize="none" autoCorrect={false} editable={!disabled} />
      <Action title="Import model file" disabled={disabled} onPress={() => void importModel().catch(e => setError(String(e)))} />
      {models.map(model => <Action key={model.id} title={`${settings.modelID === model.id ? '✓ ' : ''}${model.name} · ${(model.bytes / 1048576).toFixed(1)} MiB`} disabled={disabled} onPress={() => setSettings(current => ({ ...current, modelID: model.id }))} />)}
      <Text style={styles.body}>The default checksum is for the pinned tiny.en smoke-test model. Supply the trusted checksum for any other artifact.</Text>
      <TextInput accessibilityLabel="Language code" style={styles.input} value={settings.language ?? ''} onChangeText={language => setSettings(current => ({ ...current, language: language || null }))} editable={!disabled} placeholder="en (empty for automatic)" />
      <TextInput accessibilityLabel="Vocabulary hints" style={styles.input} value={settings.dictionary.join(', ')} onChangeText={value => setSettings(current => ({ ...current, dictionary: value.split(',') }))} editable={!disabled} placeholder="KLASS, west entrance" />
      <Text style={styles.body}>Threads (1–16)</Text><TextInput accessibilityLabel="Thread count" style={styles.input} keyboardType="number-pad" value={String(settings.threads)} onChangeText={value => setSettings(current => ({ ...current, threads: Number(value) }))} editable={!disabled} />
      <Text style={styles.body}>Maximum recording seconds (1–120)</Text><TextInput accessibilityLabel="Audio limit" style={styles.input} keyboardType="number-pad" value={String(settings.maxAudioSeconds)} onChangeText={value => setSettings(current => ({ ...current, maxAudioSeconds: Number(value) }))} editable={!disabled} />
      <Action title="Load model / apply settings" disabled={disabled || !settings.modelID} onPress={() => void operation(async () => { await Pheme.configure(settings); setMessage('Local model is ready.'); })} />
      <Text style={styles.body}>Engine: {ready ? 'ready' : 'not loaded'} · model SHA: {inspection?.model?.sha256 ?? '—'}</Text>
    </View>
    <View style={styles.card}><Text style={styles.heading}>Record or select a WAV</Text>
      <Action title={recording ? `Stop and transcribe · ${seconds}s` : 'Record on this iPhone'} disabled={busy || !ready} onPress={() => void (recording ? stopRecording() : startRecording())} />
      <Action title="Transcribe a saved WAV" disabled={disabled || !ready} onPress={() => void (async () => {
        const picked = await DocumentPicker.getDocumentAsync({ type: ['audio/wav', 'audio/x-wav'], copyToCacheDirectory: true });
        if (!picked.canceled) await operation(() => transcribe(picked.assets[0].uri, false));
      })().catch(e => setError(String(e)))} />
      {pending && <Action title="Retry saved local recording" disabled={disabled || !ready} onPress={() => void operation(() => transcribe(pending.uri, true))} />}
      {busy && <Action title="Discard result; wait for native work to finish" onPress={() => { setMessage('Discarding the result. Waiting for native work to settle…'); void Pheme.cancel(); }} />}
    </View>
    {result && <View style={styles.card}><Text style={styles.heading}>Review transcript</Text>
      <TextInput accessibilityLabel="Local transcript draft" style={[styles.input, styles.draft]} multiline value={draft} onChangeText={value => { setDraft(value); updateLocalDraft(result.transcription.run_id, value); }} />
      <Text style={styles.body}>Run: {result.transcription.run_id}{'\n'}Processing: {result.transcription.result.processing_time_ms.toFixed(1)} ms · {result.transcription.result.status}{'\n'}Artifact: {result.model.sha256}</Text>
      <Text style={styles.body}>This editable transcript is stored on the phone. It has not been finalised as an incident report.</Text>
    </View>}
    <View style={styles.card}><Text style={styles.heading}>Live diagnostic metrics</Text>
      <Text style={styles.body}>Dropped events: {observation?.observer.dropped_events ?? '—'} · retained drain payloads: {batches.length}</Text>
      <Text style={styles.body}>Metrics below are temporary diagnostics. They are not yet a durable upload queue. Power and energy are unavailable.</Text>
      <Text selectable style={styles.code}>{batches.at(-1)?.slice(0, 12000) ?? 'No measurements yet.'}</Text>
    </View>
  </ScrollView></SafeAreaView>;
}

function Action({ title, disabled, onPress }: { title: string; disabled?: boolean; onPress: () => void }) {
  return <Pressable accessibilityRole="button" disabled={disabled} onPress={onPress} style={[styles.action, disabled && styles.disabled]}><Text style={styles.actionText}>{title}</Text></Pressable>;
}
const styles = StyleSheet.create({
  safe: { flex: 1, backgroundColor: '#fff' }, content: { padding: 20, gap: 16 }, link: { color: '#176b5b', fontWeight: '600' },
  title: { fontSize: 25, fontWeight: '700', color: '#172033' }, heading: { fontSize: 18, fontWeight: '600', color: '#172033' },
  body: { fontSize: 13, lineHeight: 20, color: '#667085' }, status: { color: '#176b5b', fontSize: 14 },
  card: { padding: 16, borderWidth: 1, borderColor: '#e4e7ec', borderRadius: 12, gap: 12 },
  input: { padding: 12, borderWidth: 1, borderColor: '#d0d5dd', borderRadius: 8, color: '#172033', fontSize: 14 }, draft: { minHeight: 120, textAlignVertical: 'top' },
  action: { padding: 13, borderRadius: 8, backgroundColor: '#176b5b' }, actionText: { color: '#fff', fontWeight: '600', textAlign: 'center' },
  disabled: { opacity: 0.4 }, error: { padding: 12, color: '#b42318', backgroundColor: '#fff4ed', borderRadius: 8 }, code: { fontSize: 10, fontFamily: 'monospace', color: '#344054' },
});
