import { Link, useFocusEffect } from "expo-router";
import { useCallback, useEffect, useRef, useState } from "react";
import { ActivityIndicator, Animated, AppState, Dimensions, FlatList, KeyboardAvoidingView, PanResponder, Platform, Pressable, StyleSheet, Text, TextInput, View } from "react-native";
import { useAudioStream, requestRecordingPermissionsAsync, setAudioModeAsync } from "expo-audio";
import type { AudioStreamBuffer } from "expo-audio";
import * as DocumentPicker from "expo-document-picker";
import * as Speech from "expo-speech";
import MaterialIcons from "@expo/vector-icons/MaterialIcons";
import { SafeAreaView } from "react-native-safe-area-context";
import { cancelTurn, getReadiness, getTurn, startTurn, submitTurn, resetChat } from "../src/onDeviceChat";
import Pheme from "../modules/pheme-native/src/PhemeNativeModule";
import type { Attachment, Readiness, Turn } from "../src/types";
import { pcm16ToWav } from "../src/wav";
import { pendingFile, purgeExpiredRecording, readPendingRecording, removePendingRecording, saveRecording, updatePendingRecording, type PendingRecording } from "../src/recordingStore";
import { listRecentChats, saveChat } from "../src/chatStore";

const colors = { ink: "#172033", muted: "#667085", line: "#e4e7ec", panel: "#f7f8fa", accent: "#176b5b", user: "#e4f3ef" };
const DRAWER_WIDTH = Dimensions.get("window").width * 0.82;

export default function ChatScreen() {
  const [active, setActive] = useState<Turn | null>(null);
  const [history, setHistory] = useState<Turn[]>([]);
  const [recentChats, setRecentChats] = useState<Turn[]>([]);
  const [menuOpen, setMenuOpen] = useState(false);
  const [pendingRecovery, setPendingRecovery] = useState(false);
  const drawerX = useRef(new Animated.Value(-DRAWER_WIDTH)).current;
  const [draft, setDraft] = useState("");
  const [attachments, setAttachments] = useState<Attachment[]>([]);
  const [readiness, setReadiness] = useState<Readiness | null>(null);
  const [busy, setBusy] = useState(false);
  const [recording, setRecording] = useState(false);
  const [recordingSeconds, setRecordingSeconds] = useState(0);
  const [error, setError] = useState("");
  const [typingDots, setTypingDots] = useState("...");
  const abortRef = useRef<AbortController | null>(null);
  const chunksRef = useRef<ArrayBuffer[]>([]);
  const sampleRateRef = useRef(16000);
  const channelsRef = useRef(1);
  const recordingRef = useRef(false);
  const spokenTurnRef = useRef("");
  const previousStatusRef = useRef<Turn["status"] | null>(null);
  const pendingRecordingRef = useRef<PendingRecording | null>(null);
  const pollingTurnRef = useRef<string | null>(null);
  const busyRef = useRef(false);
  const recordedBytes = useRef(0);
  const recordingLimit = useRef(120);

  const audio = useAudioStream({
    sampleRate: 16000,
    channels: 1,
    encoding: "int16",
    onBuffer: (buffer: AudioStreamBuffer) => {
      if (!recordingRef.current) return;
      // Copy native memory before the audio callback returns.
      const limit = buffer.sampleRate * buffer.channels * 2 * recordingLimit.current;
      const take = Math.min(buffer.data.byteLength, Math.max(0, limit - recordedBytes.current));
      if (take) chunksRef.current.push(new Uint8Array(buffer.data).slice(0, take).buffer);
      recordedBytes.current += take;
      sampleRateRef.current = buffer.sampleRate;
      channelsRef.current = buffer.channels;
    },
  });

  function openDrawer() {
    setMenuOpen(true);
    Animated.spring(drawerX, { toValue: 0, speed: 24, bounciness: 0, useNativeDriver: true }).start();
  }

  function closeDrawer() {
    Animated.spring(drawerX, { toValue: -DRAWER_WIDTH, speed: 28, bounciness: 0, useNativeDriver: true }).start(({ finished }) => {
      if (finished) setMenuOpen(false);
    });
  }

  const drawerPanResponder = useRef(PanResponder.create({
    onMoveShouldSetPanResponder: (_, gesture) => gesture.dx > 10 && Math.abs(gesture.dx) > Math.abs(gesture.dy),
    onPanResponderMove: (_, gesture) => drawerX.setValue(Math.max(-DRAWER_WIDTH, Math.min(0, gesture.dx))),
    onPanResponderRelease: (_, gesture) => gesture.dx > DRAWER_WIDTH * 0.25 ? closeDrawer() : openDrawer(),
  })).current;
  const edgePanResponder = useRef(PanResponder.create({
    onStartShouldSetPanResponder: (_, gesture) => gesture.x0 < 24,
    onMoveShouldSetPanResponder: (_, gesture) => gesture.x0 < 24 && gesture.dx > 8 && Math.abs(gesture.dx) > Math.abs(gesture.dy),
    onPanResponderGrant: () => setMenuOpen(true),
    onPanResponderMove: (_, gesture) => drawerX.setValue(Math.max(-DRAWER_WIDTH, Math.min(0, -DRAWER_WIDTH + gesture.dx))),
    onPanResponderRelease: (_, gesture) => gesture.dx > DRAWER_WIDTH * 0.25 ? openDrawer() : closeDrawer(),
  })).current;

  useEffect(() => {
    setRecentChats(listRecentChats());
    purgeExpiredRecording();
    const pending = readPendingRecording('local');
    if (pending) pendingRecordingRef.current = pending;
    if (pending) setPendingRecovery(true);
  }, []);
  useFocusEffect(useCallback(() => {
    let stopped = false;
    setError(""); setBusy(true); busyRef.current = true;
    getReadiness().then(value => {
      if (!stopped) { setReadiness(value); recordingLimit.current = value.maxAudioSeconds ?? 120; }
    })
      .catch(e => { if (!stopped) setError(e.message); })
      .finally(() => { busyRef.current = false; if (!stopped) setBusy(false); });
    return () => { stopped = true; };
  }, []));
  useEffect(() => {
    const subscription = AppState.addEventListener('change', state => {
      if (state === 'active') return;
      if (recordingRef.current) {
        recordingRef.current = false;
        audio.stream.stop();
        chunksRef.current = []; recordedBytes.current = 0;
        setRecording(false);
        void setAudioModeAsync({ allowsRecording: false, playsInSilentMode: true });
        setError('Recording interrupted. Record again when the app is active.');
      }
      void Speech.stop();
      void Pheme.cancel().catch(() => {});
    });
    return () => { subscription.remove(); audio.stream.stop(); void Speech.stop(); };
  }, [audio.stream]);
  useEffect(() => {
    if (!active || active.status === "transcribing" || active.id.startsWith("pending-")) return;
    saveChat(active);
    setRecentChats(listRecentChats());
  }, [active]);
  useEffect(() => {
    if (active?.status !== "generating") return;
    let step = 0;
    const timer = setInterval(() => { step = (step + 1) % 4; setTypingDots(".".repeat(step || 1)); }, 350);
    return () => clearInterval(timer);
  }, [active?.status]);
  useEffect(() => {
    if (active?.status !== "generating" || pollingTurnRef.current === active.id) return;
    void pollTurn(active.id);
  }, [active?.status, active?.id]);
  useEffect(() => {
    if (!recording) return;
    const started = Date.now();
    const timer = setInterval(() => setRecordingSeconds(Math.floor((Date.now() - started) / 1000)), 250);
    const timeout = setTimeout(() => void stopRecording(), recordingLimit.current * 1000);
    return () => { clearInterval(timer); clearTimeout(timeout); };
  }, [recording]);

  async function speakReply(text: string) {
    try {
      await Speech.stop();
      const voices = await Speech.getAvailableVoicesAsync();
      const englishVoice = voices.find((voice) => voice.language.toLowerCase().startsWith("en"));
      await new Promise((resolve) => setTimeout(resolve, 150));
      Speech.speak(text, {
        voice: englishVoice?.identifier,
        rate: 0.95,
        onError: () => setError("Phone text-to-speech could not start. Check that a system voice is installed.")
      });
    } catch {
      setError("Phone text-to-speech could not start. Check that a system voice is installed.");
    }
  }

  useEffect(() => {
    const previousStatus = previousStatusRef.current;
    previousStatusRef.current = active?.status ?? null;
    if (active?.status !== "completed" || previousStatus !== "generating" || !active.reply || spokenTurnRef.current === active.id) return;
    spokenTurnRef.current = active.id;
    void speakReply(active.reply);
  }, [active?.status, active?.id, active?.reply]);

  function updateFromEvent(event: { event: string; data: Record<string, unknown> }) {
    const pending = pendingRecordingRef.current;
    if (pending && event.data.turn_id) updatePendingRecording({ turnId: String(event.data.turn_id), status: "saved" }, 'local');
    setActive((current) => {
      // Keep the local turn ID across progress events. Later events may omit it;
      // falling back to the render-time `active` value can restore the temporary ID
      // and make the approval request target a nonexistent turn.
      const id = String(event.data.turn_id ?? current?.id ?? "");
      const next = { ...(current ?? { id, status: "transcribing" as const, transcript: "", approvedText: null, reply: "", attachments }), id: id || current?.id || "" };
      if (event.event === "transcript.ready") { setBusy(false); setPendingRecovery(false); if (pending) { removePendingRecording('local'); pendingRecordingRef.current = null; } setDraft(String(event.data.text ?? "")); return { ...next, status: "awaiting_review", transcript: String(event.data.text ?? "") }; }
      if (event.event === "question.approved") { setBusy(true); setDraft(""); return { ...next, status: "generating", approvedText: String(event.data.text ?? "") }; }
      if (event.event === "reply.started") return { ...next, status: "generating" };
      if (event.event === "reply.delta") return { ...next, status: "generating", reply: next.reply + String(event.data.text ?? "") };
      if (event.event === "reply.completed") { setBusy(false); return { ...next, status: "completed", reply: String(event.data.text ?? next.reply) }; }
      if (event.event === "turn.cancelled") { setBusy(false); return { ...next, status: "cancelled" }; }
      if (event.event === "turn.failed" || event.event === "reply.failed") {
        setBusy(false);
        const failure = event.data.error as { message?: string } | undefined;
        if (pending) updatePendingRecording({ status: "failed", error: failure?.message ?? "The turn failed." }, 'local');
        return { ...next, status: "failed", error: failure?.message ?? "The turn failed." };
      }
      return next;
    });
  }

  async function runTurn(input: string | ReturnType<typeof pendingFile>, pending: PendingRecording | null = null) {
    if (busyRef.current) return;
    busyRef.current = true;
    if (active?.status === "completed") {
      setHistory((current) => current.some((turn) => turn.id === active.id) ? current : [...current, active]);
    }
    setError(""); setBusy(true); setDraft(typeof input === "string" ? input : ""); setActive({ id: `pending-${Date.now()}`, status: "transcribing", transcript: typeof input === "string" ? input : "", approvedText: null, reply: "", attachments });
    const controller = new AbortController(); abortRef.current = controller;
    if (pending) { pendingRecordingRef.current = pending; setPendingRecovery(false); updatePendingRecording({ status: "saved" }, 'local'); }
    try { await startTurn(input, updateFromEvent, controller.signal); }
    catch (e) {
      if (!controller.signal.aborted) {
        const message = e instanceof Error ? e.message : "Could not send question.";
        setError(message.toLowerCase().includes("inference") ? "Another report is still processing. Check the input bar." : message);
        setActive((current) => current ? { ...current, status: "failed", error: message } : current);
        if (pending) { updatePendingRecording({ status: "failed", error: message }, 'local'); setPendingRecovery(true); }
      }
    }
    finally { busyRef.current = false; setBusy(false); abortRef.current = null; }
  }

  async function send() {
    const text = draft.trim();
    if (!text || busyRef.current || busy || !readiness?.reply.ready) return;
    await runTurn(text);
  }

  async function startRecording() {
    if (busyRef.current || busy || recording || !readiness?.stt.ready) return;
    if (active?.status === "awaiting_review" && !draft.trim()) {
      // An empty reviewed draft means the user discarded the transcript.
      // Cancel that local turn before starting a fresh recording.
      const discardedTurnId = active.id;
      setActive(null);
      setDraft("");
      setBusy(false);
      try { await cancelTurn(discardedTurnId); } catch { /* it may already be gone */ }
    } else if (active && ["transcribing", "awaiting_review", "generating"].includes(active.status)) {
      setError("Finish the current report in the input bar before starting another one.");
      return;
    }
    setError("");
    const permission = await requestRecordingPermissionsAsync();
    if (!permission.granted) { setError("Microphone permission is required. Enable it in the phone settings."); return; }
    try {
      // Keep the microphone usable while iOS ReplayKit/Screen Recording is active.
      await setAudioModeAsync({ allowsRecording: true, playsInSilentMode: true, interruptionMode: "mixWithOthers" });
      chunksRef.current = [];
      recordedBytes.current = 0;
      sampleRateRef.current = 16000;
      channelsRef.current = 1;
      recordingRef.current = true;
      await audio.stream.start();
      setRecordingSeconds(0);
      setRecording(true);
    } catch (e) {
      recordingRef.current = false;
      setRecording(false);
      try { audio.stream.stop(); } catch { /* stream may not have started */ }
      try { await setAudioModeAsync({ allowsRecording: false, playsInSilentMode: true, interruptionMode: "mixWithOthers" }); } catch { /* keep the original error */ }
      setError(e instanceof Error ? e.message : "Could not start recording.");
    }
  }

  async function stopRecording() {
    if (!recordingRef.current) return;
    try {
      audio.stream.stop();
      // Stop updating the UI immediately. Native transcription can
      // take much longer and must not keep the button looking like it listens.
      recordingRef.current = false;
      setRecording(false);
      await new Promise((resolve) => setTimeout(resolve, 150));
      const chunks = chunksRef.current;
      if (!chunks.length) { setError("No microphone audio was captured. Please check microphone permission and try again."); return; }
      const hasSignal = chunks.some((chunk) => {
        const samples = new Int16Array(chunk);
        for (let index = 0; index < samples.length; index += 1) {
          if (Math.abs(samples[index]) >= 256) return true;
        }
        return false;
      });
      if (!hasSignal) {
        setError("The recording contains no audible microphone signal. Enable microphone access for this app and try again.");
        return;
      }
      const wav = pcm16ToWav(chunks, sampleRateRef.current, channelsRef.current);
      const durationSeconds = chunks.reduce((total, chunk) => total + chunk.byteLength, 0) / (sampleRateRef.current * channelsRef.current * 2);
      if (durationSeconds > recordingLimit.current) { setError(`Local recordings are limited to ${recordingLimit.current} seconds.`); return; }
      // Persist before inference so failure leaves a retryable local recording.
      const pending = saveRecording(wav, durationSeconds, 'local');
      pendingRecordingRef.current = pending;
      await runTurn(pendingFile(pending), pending);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Could not stop recording.");
    } finally {
      recordingRef.current = false;
      setRecording(false);
      try { await setAudioModeAsync({ allowsRecording: false, playsInSilentMode: true, interruptionMode: "mixWithOthers" }); } catch { /* the next start will reconfigure it */ }
    }
  }

  async function retrySavedRecording() {
    const pending = readPendingRecording('local');
    if (!pending || busy) return;
    setError("");
    setPendingRecovery(false);
    await runTurn(pendingFile(pending), pending);
  }

  async function approve() {
    if (busyRef.current || !active || active.status !== "awaiting_review" || !draft.trim()) return;
    if (!active.id || active.id.startsWith("pending-")) { setError("The local turn is not ready. Please wait and try again."); return; }
    const turnID = active.id;
    busyRef.current = true;
    setBusy(true); setError("");
    setActive((current) => current?.id === turnID ? { ...current, status: "generating", transcript: "", approvedText: draft.trim() } : current);
    try { await submitTurn(turnID, draft.trim()); await pollTurn(turnID); }
    catch (e) {
      setBusy(false);
      const message = e instanceof Error ? e.message : "Could not approve question.";
      setError(message.toLowerCase().includes("inference") ? "Another report is still processing. Check the input bar and try again when it finishes." : message);
    }
    finally { busyRef.current = false; }
  }

  async function pollTurn(turnID: string) {
    if (pollingTurnRef.current === turnID) return;
    pollingTurnRef.current = turnID;
    try {
      for (let attempt = 0; attempt < 180; attempt += 1) {
        const turn = await getTurn(turnID);
        const status = turn.status === "awaiting_review" ? "generating" : turn.status;
        setActive((current) => current?.id === turnID ? {
          ...current,
          ...turn,
          status,
          transcript: turn.transcript || current.transcript,
          approvedText: turn.approvedText ?? current.approvedText,
          reply: turn.reply || current.reply,
        } : current);
        if (["completed", "failed", "cancelled"].includes(turn.status)) {
          setBusy(false);
          return;
        }
        await new Promise((resolve) => setTimeout(resolve, 1000));
      }
      setBusy(false);
      setError("The reply is taking longer than expected. Tap Read status to check again.");
    } catch (e) {
      setBusy(false);
      setError(e instanceof Error ? e.message : "Could not read the turn status.");
    } finally {
      pollingTurnRef.current = null;
    }
  }

  async function stop() {
    abortRef.current?.abort();
    if (active) { try { await cancelTurn(active.id); } catch { /* already stopped locally */ } }
    // Native cancellation settles asynchronously. The originating promise owns
    // the busy flag until Pheme has released its admitted operation.
  }

  async function recover() {
    if (!active) return;
    try { setActive(await getTurn(active.id)); }
    catch (e) { setError(e instanceof Error ? e.message : "Could not recover turn."); }
  }

  async function openRecentChat(chat: Turn) {
    if (busy || busyRef.current) return;
    void Speech.stop();
    busyRef.current = true; setBusy(true);
    try {
      await resetChat();
      setHistory([chat]); setActive(null);
      setDraft(chat.status === "awaiting_review" ? chat.transcript : "");
      setError("Saved chat is read-only. New messages start a fresh local conversation.");
      closeDrawer();
    } catch (e) { setError(e instanceof Error ? e.message : 'Could not open saved chat.'); }
    finally { busyRef.current = false; setBusy(false); }
  }

  async function startNewChat() {
    if (busy || busyRef.current) return;
    void Speech.stop();
    busyRef.current = true; setBusy(true);
    try {
      await resetChat();
      setActive(null); setHistory([]); setDraft(""); setError(""); setAttachments([]); closeDrawer();
    } catch (e) { setError(e instanceof Error ? e.message : 'Could not reset local conversation.'); }
    finally { busyRef.current = false; setBusy(false); }
  }

  async function pickFile() {
    const result = await DocumentPicker.getDocumentAsync({ type: "*/*", multiple: true, copyToCacheDirectory: true });
    if (!result.canceled) setAttachments((current) => [...current, ...result.assets.map((item) => ({ uri: item.uri, name: item.name, mimeType: item.mimeType, size: item.size }))]);
  }

  const displayTurns = [...history, ...(active ? [active] : [])];
  return <SafeAreaView style={styles.safe}><KeyboardAvoidingView style={styles.screen} behavior={Platform.OS === "ios" ? "padding" : "height"} keyboardVerticalOffset={0}>
    <View style={styles.header}>
      <Pressable accessibilityLabel="Open recent chats" onPress={openDrawer} style={styles.menuButton}><MaterialIcons name="menu" size={27} color={colors.ink} /></Pressable>
      <View style={styles.headerCopy}>
        <Text style={styles.title}>Incident Reporting Agent</Text>
        <Link href="/local-transcription" disabled={busy || recording} style={styles.action}>On-device model settings</Link>
        <Text style={styles.subtitle}>{readiness?.stt.ready ? `ASR: ${readiness.stt.name}` : "Loading local ASR model..."}</Text>
        <Text style={styles.subtitle}>{readiness?.reply.ready ? `Reasoning: ${readiness.reply.name}` : "Loading local reasoning model..."}</Text>
        <Text style={styles.subtitle}>Inference runs on this iPhone. Replies are not saved incident reports.</Text>
      </View>
      <View style={[styles.dot, { backgroundColor: readiness?.stt.ready && readiness?.reply.ready ? colors.accent : "#d0d5dd" }]} />
    </View>
    <FlatList data={displayTurns} keyExtractor={(item) => item.id} contentContainerStyle={styles.messages} renderItem={({ item }) => { const question = item.approvedText; return <View style={styles.turn}>{question ? <View style={[styles.bubble, styles.userBubble]}><Text style={styles.label}>You</Text><Text style={styles.body}>{question}</Text></View> : null}{item.status === "awaiting_review" && item.transcript ? <View style={[styles.bubble, styles.transcriptBubble]}><Text style={styles.label}>Transcript</Text><Text style={styles.body}>{item.transcript}</Text></View> : null}{item.status === "transcribing" ? <View style={[styles.bubble, styles.assistantBubble, styles.statusBubble]}><ActivityIndicator color={colors.accent} /><Text style={styles.statusText}>Processing report…</Text></View> : null}{item.status === "generating" ? <View style={[styles.bubble, styles.assistantBubble]}><Text style={styles.label}>Incident agent</Text>{item.reply ? <Text style={styles.body}>{item.reply}</Text> : <Text style={styles.typing}>{typingDots}</Text>}{item.reply ? <Pressable onPress={() => void speakReply(item.reply)}><Text style={styles.action}>🔊 Speak</Text></Pressable> : null}</View> : null}{item.status === "completed" ? <View style={[styles.bubble, styles.assistantBubble]}><Text style={styles.label}>Incident agent</Text><Text style={styles.body}>{item.reply || "Reply completed, but no reply text was returned."}</Text>{item.reply ? <Pressable onPress={() => void speakReply(item.reply)}><Text style={styles.action}>🔊 Speak</Text></Pressable> : null}</View> : null}{item.status === "failed" ? <View style={[styles.bubble, styles.failureBubble]}><Text style={styles.label}>Transcription failed</Text><Text style={styles.failureText}>{item.error ?? "The report could not be transcribed."}</Text><Text style={styles.statusText}>Check the microphone permission and try recording again.</Text></View> : null}{item.status === "cancelled" ? <View style={[styles.bubble, styles.assistantBubble]}><Text style={styles.statusText}>Report cancelled.</Text></View> : null}</View>; }} ListEmptyComponent={<View style={styles.empty}><Pressable onPress={recording ? stopRecording : startRecording} style={[styles.micButton, recording && styles.micButtonRecording]} disabled={busy && !recording}><MaterialIcons name={recording ? "stop" : "mic"} size={48} color="#fff" /></Pressable></View>} />
    {recording ? <View style={styles.recording}><Text style={styles.recordingText}>● Listening… {recordingSeconds}s</Text></View> : null}
    {attachments.length ? <View style={styles.chips}>{attachments.map((file, index) => <Pressable key={`${file.uri}-${index}`} onPress={() => setAttachments((items) => items.filter((_, i) => i !== index))} style={styles.chip}><Text numberOfLines={1} style={styles.chipText}>📎 {file.name}  ×</Text></Pressable>)}</View> : null}
    {error ? <View style={styles.error}><Text style={styles.errorText}>{error}</Text>{active ? <Pressable onPress={recover}><Text style={styles.action}>Read status</Text></Pressable> : null}</View> : null}
    {pendingRecovery ? <View style={styles.recovery}><Text style={styles.recoveryText}>A previous local recording is ready to retry.</Text><Pressable onPress={() => void retrySavedRecording()} disabled={busy}><Text style={styles.action}>Retry transcription</Text></Pressable></View> : null}
    {busy && active?.status === "generating" ? <Pressable accessibilityRole="button" onPress={() => void stop()}><Text style={styles.action}>Cancel local reply</Text></Pressable> : null}
    {busy && active?.status === "transcribing" ? <View style={styles.processing}><ActivityIndicator color={colors.accent} /><Text style={styles.statusText}>Processing your report…</Text></View> : null}
    <Text style={styles.footnote}>{recording ? "Speak clearly, then tap the button again to review the transcription." : active?.status === "awaiting_review" ? "Check the input bar, edit the transcript, then tap ↑ to send." : active?.status === "generating" ? "Incident agent is responding…" : "Start an incident report · Ask by typing or speaking"}</Text>
    {!active || active.status === "awaiting_review" || active.status === "completed" || active.status === "failed" || active.status === "cancelled" ? <View style={styles.composer}><Pressable onPress={pickFile} style={styles.icon}><Text>＋</Text></Pressable><Pressable accessibilityLabel={recording ? "Stop speaking" : "Speak"} onPress={recording ? stopRecording : startRecording} style={[styles.icon, styles.speakButton]} disabled={busy && !recording}><MaterialIcons name={recording ? "stop" : "mic"} size={21} color={recording ? "#b54708" : colors.accent} /></Pressable><TextInput value={draft} onChangeText={setDraft} placeholder={active?.status === "awaiting_review" ? "Edit the transcript, then tap ↑ to send…" : "Or type an incident question…"} placeholderTextColor="#98a2b3" multiline style={styles.input} editable={!busy || active?.status === "awaiting_review"} /><Pressable onPress={active?.status === "awaiting_review" ? approve : busy ? stop : send} style={[styles.send, !draft.trim() && !busy && styles.disabled]}><MaterialIcons name={busy ? "stop" : "arrow-upward"} size={22} color="#fff" /></Pressable></View> : null}
  </KeyboardAvoidingView><View {...edgePanResponder.panHandlers} style={styles.edgeSwipe} /><View pointerEvents={menuOpen ? "auto" : "none"} style={styles.drawerLayer}><Animated.View style={[styles.scrim, { opacity: drawerX.interpolate({ inputRange: [-DRAWER_WIDTH, 0], outputRange: [0, 1], extrapolate: "clamp" }) }]}><Pressable style={StyleSheet.absoluteFill} onPress={closeDrawer} /></Animated.View><Animated.View {...drawerPanResponder.panHandlers} style={[styles.drawer, { transform: [{ translateX: drawerX }] }]}><View style={styles.drawerHeader}><Text style={styles.drawerTitle}>Recent chats</Text><Pressable onPress={closeDrawer}><MaterialIcons name="close" size={24} color={colors.ink} /></Pressable></View><Pressable style={styles.newChat} onPress={startNewChat}><MaterialIcons name="add" size={20} color="#fff" /><Text style={styles.newChatText}>New chat</Text></Pressable><FlatList data={recentChats} keyExtractor={(item) => item.id} ListEmptyComponent={<Text style={styles.emptyChats}>No recent chats yet.</Text>} renderItem={({ item }) => <Pressable style={styles.chatRow} onPress={() => openRecentChat(item)}><Text numberOfLines={1} style={styles.chatTitle}>{item.approvedText || item.transcript || "Untitled voice report"}</Text><Text style={styles.chatStatus}>{item.status === "completed" ? "Completed" : item.status === "awaiting_review" ? "Needs review" : item.status}</Text></Pressable>} /></Animated.View></View></SafeAreaView>;
}

const styles = StyleSheet.create({ safe: { flex: 1, backgroundColor: "#fff" }, screen: { flex: 1 }, header: { padding: 16, borderBottomWidth: 1, borderBottomColor: colors.line, flexDirection: "row", alignItems: "center", gap: 12 }, menuButton: { width: 38, height: 38, alignItems: "center", justifyContent: "center" }, headerCopy: { flex: 1 }, title: { fontSize: 21, fontWeight: "700", color: colors.ink }, subtitle: { fontSize: 12, color: colors.muted, marginTop: 4 }, dot: { width: 10, height: 10, borderRadius: 5 }, messages: { padding: 16, gap: 14, flexGrow: 1 }, turn: { gap: 8 }, bubble: { borderRadius: 18, padding: 14, maxWidth: "90%" }, userBubble: { alignSelf: "flex-end", backgroundColor: colors.user }, assistantBubble: { alignSelf: "flex-start", backgroundColor: colors.panel }, transcriptBubble: { alignSelf: "flex-end", backgroundColor: colors.panel }, failureBubble: { alignSelf: "flex-start", backgroundColor: "#fff4ed" }, label: { fontSize: 11, fontWeight: "700", color: colors.muted, marginBottom: 5 }, body: { fontSize: 16, lineHeight: 23, color: colors.ink }, failureText: { color: "#b54708", fontSize: 15, lineHeight: 21, marginBottom: 6 }, action: { color: colors.accent, fontWeight: "700", marginTop: 10 }, empty: { alignItems: "center", justifyContent: "center", flex: 1, minHeight: 260 }, emptyChats: { color: colors.muted, padding: 16 }, emptyTitle: { fontSize: 18, fontWeight: "700", color: colors.ink, marginBottom: 5 }, recording: { margin: 12, padding: 14, borderRadius: 16, backgroundColor: "#fff4ed", gap: 10 }, recordingText: { color: "#b54708", fontWeight: "700", textAlign: "center" }, processing: { flexDirection: "row", alignItems: "center", justifyContent: "center", gap: 8, paddingVertical: 6 }, statusBubble: { flexDirection: "row", alignItems: "center", gap: 8 }, statusText: { color: colors.muted, fontSize: 13 }, typing: { color: colors.muted, fontSize: 26, letterSpacing: 3, lineHeight: 30 }, review: { margin: 12, padding: 14, borderRadius: 16, borderWidth: 1, borderColor: "#b7dfd4", backgroundColor: "#f4fbf9" }, reviewInput: { minHeight: 70, maxHeight: 130, fontSize: 16, color: colors.ink, paddingVertical: 8 }, primary: { backgroundColor: colors.accent, padding: 13, alignItems: "center", borderRadius: 12 }, primaryText: { color: "#fff", fontWeight: "700" }, chips: { flexDirection: "row", flexWrap: "wrap", gap: 8, paddingHorizontal: 12 }, chip: { backgroundColor: colors.panel, borderRadius: 20, paddingHorizontal: 10, paddingVertical: 8, maxWidth: "90%" }, chipText: { color: colors.ink, fontSize: 12 }, error: { margin: 12, padding: 10, borderRadius: 10, backgroundColor: "#fff4ed" }, errorText: { color: "#b54708", fontSize: 13 }, recovery: { margin: 12, padding: 10, borderRadius: 10, backgroundColor: "#f4fbf9", borderWidth: 1, borderColor: "#b7dfd4" }, recoveryText: { color: colors.ink, fontSize: 13 }, micButton: { alignSelf: "center", width: 150, height: 150, borderRadius: 75, backgroundColor: colors.accent, alignItems: "center", justifyContent: "center", marginVertical: 8, shadowColor: colors.accent, shadowOpacity: 0.2, shadowRadius: 10, elevation: 4 }, micButtonRecording: { backgroundColor: "#b54708" }, composer: { margin: 12, borderWidth: 1, borderColor: colors.line, borderRadius: 20, minHeight: 54, flexDirection: "row", alignItems: "center", padding: 7, gap: 6 }, icon: { width: 38, height: 38, borderRadius: 19, backgroundColor: colors.panel, alignItems: "center", justifyContent: "center" }, speakButton: { borderWidth: 1, borderColor: colors.line }, input: { flex: 1, maxHeight: 110, fontSize: 16, color: colors.ink, paddingHorizontal: 6, paddingVertical: 9 }, send: { width: 38, height: 38, borderRadius: 19, backgroundColor: colors.accent, alignItems: "center", justifyContent: "center" }, disabled: { opacity: 0.4 }, footnote: { fontSize: 13, color: colors.muted, textAlign: "center", marginTop: 2, marginBottom: 2, paddingHorizontal: 12 }, edgeSwipe: { position: "absolute", left: 0, top: 0, bottom: 0, width: 24, zIndex: 20 }, drawerLayer: { ...StyleSheet.absoluteFill, zIndex: 30, elevation: 30 }, scrim: { ...StyleSheet.absoluteFill, backgroundColor: "rgba(0,0,0,0.35)" }, drawer: { position: "absolute", left: 0, top: 0, bottom: 0, width: DRAWER_WIDTH, backgroundColor: "#fff", paddingTop: 58, paddingHorizontal: 18, shadowColor: "#000", shadowOpacity: 0.18, shadowOffset: { width: 5, height: 0 }, shadowRadius: 12, elevation: 12 }, drawerHeader: { flexDirection: "row", alignItems: "center", justifyContent: "space-between", paddingBottom: 18, borderBottomWidth: 1, borderBottomColor: colors.line }, drawerTitle: { fontSize: 22, fontWeight: "700", color: colors.ink }, newChat: { marginVertical: 16, borderRadius: 12, backgroundColor: colors.accent, padding: 13, flexDirection: "row", alignItems: "center", justifyContent: "center", gap: 8 }, newChatText: { color: "#fff", fontWeight: "700" }, chatRow: { paddingVertical: 14, borderBottomWidth: 1, borderBottomColor: colors.line }, chatTitle: { color: colors.ink, fontSize: 15, fontWeight: "600" }, chatStatus: { color: colors.muted, fontSize: 12, marginTop: 5, textTransform: "capitalize" } });
