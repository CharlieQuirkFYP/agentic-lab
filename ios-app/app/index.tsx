import { useEffect, useRef, useState } from "react";
import { ActivityIndicator, FlatList, KeyboardAvoidingView, Platform, Pressable, StyleSheet, Text, TextInput, View } from "react-native";
import { useAudioStream, requestRecordingPermissionsAsync, setAudioModeAsync } from "expo-audio";
import type { AudioStreamBuffer } from "expo-audio";
import * as DocumentPicker from "expo-document-picker";
import * as Speech from "expo-speech";
import MaterialIcons from "@expo/vector-icons/MaterialIcons";
import { SafeAreaView } from "react-native-safe-area-context";
import { cancelTurn, getReadiness, getTurn, startTurn, submitTurn } from "../src/api";
import type { Attachment, Readiness, Turn } from "../src/types";
import { pcm16ToWav } from "../src/wav";

const colors = { ink: "#172033", muted: "#667085", line: "#e4e7ec", panel: "#f7f8fa", accent: "#176b5b", user: "#e4f3ef" };

export default function ChatScreen() {
  const [active, setActive] = useState<Turn | null>(null);
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
  const recordingRef = useRef(false);
  const spokenTurnRef = useRef("");

  const audio = useAudioStream({
    sampleRate: 16000,
    channels: 1,
    encoding: "int16",
    onBuffer: (buffer: AudioStreamBuffer) => {
      if (!recordingRef.current) return;
      chunksRef.current.push(buffer.data);
      sampleRateRef.current = buffer.sampleRate;
    },
  });

  useEffect(() => {
    getReadiness().then((value) => {
      setReadiness(value);
      const current = value.current_turn;
      if (!current || !["transcribing", "awaiting_review", "generating"].includes(current.status)) return;
      const restored: Turn = {
        id: current.turn_id,
        status: current.status,
        transcript: current.transcript ?? "",
        approvedText: current.approved_text ?? null,
        reply: current.reply ?? "",
        attachments: [],
      };
      setActive(restored);
      if (restored.status === "awaiting_review") setDraft(restored.transcript);
    }).catch((e) => setError(e.message));
  }, []);
  useEffect(() => {
    if (active?.status !== "generating") return;
    let step = 0;
    const timer = setInterval(() => { step = (step + 1) % 4; setTypingDots(".".repeat(step || 1)); }, 350);
    return () => clearInterval(timer);
  }, [active?.status]);
  useEffect(() => {
    if (!recording) return;
    const started = Date.now();
    const timer = setInterval(() => setRecordingSeconds(Math.floor((Date.now() - started) / 1000)), 250);
    return () => clearInterval(timer);
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
    if (active?.status !== "completed" || !active.reply || spokenTurnRef.current === active.id) return;
    spokenTurnRef.current = active.id;
    void speakReply(active.reply);
  }, [active?.status, active?.id, active?.reply]);

  function updateFromEvent(event: { event: string; data: Record<string, unknown> }) {
    setActive((current) => {
      // Keep the server-issued turn ID across SSE events. Later events may omit it;
      // falling back to the render-time `active` value can restore the temporary ID
      // and make the approval request target a nonexistent turn.
      const id = String(event.data.turn_id ?? current?.id ?? "");
      const next = { ...(current ?? { id, status: "transcribing" as const, transcript: "", approvedText: null, reply: "", attachments }), id: id || current?.id || "" };
      if (event.event === "transcript.ready") { setBusy(false); setDraft(String(event.data.text ?? "")); return { ...next, status: "awaiting_review", transcript: String(event.data.text ?? "") }; }
      if (event.event === "question.approved") { setBusy(true); setDraft(""); return { ...next, status: "generating", approvedText: String(event.data.text ?? "") }; }
      if (event.event === "reply.started") return { ...next, status: "generating" };
      if (event.event === "reply.delta") return { ...next, status: "generating", reply: next.reply + String(event.data.text ?? "") };
      if (event.event === "reply.completed") { setBusy(false); return { ...next, status: "completed", reply: String(event.data.text ?? next.reply) }; }
      if (event.event === "turn.cancelled") { setBusy(false); return { ...next, status: "cancelled" }; }
      if (event.event === "turn.failed" || event.event === "reply.failed") { setBusy(false); return { ...next, status: "failed", error: String((event.data.error as { message?: string })?.message ?? "The turn failed") }; }
      return next;
    });
  }

  async function runTurn(input: string | Uint8Array) {
    setError(""); setBusy(true); setDraft(typeof input === "string" ? input : ""); setActive({ id: `pending-${Date.now()}`, status: "transcribing", transcript: typeof input === "string" ? input : "", approvedText: null, reply: "", attachments });
    const controller = new AbortController(); abortRef.current = controller;
    try { await startTurn(input, updateFromEvent, controller.signal); }
    catch (e) {
      if (!controller.signal.aborted) {
        const message = e instanceof Error ? e.message : "Could not send question.";
        setError(message.toLowerCase().includes("inference") ? "Another report is still processing. Check the input bar." : message);
        setActive((current) => current ? { ...current, status: "failed", error: message } : current);
      }
    }
    finally { setBusy(false); abortRef.current = null; }
  }

  async function send() {
    const text = draft.trim();
    if (!text || busy) return;
    await runTurn(text);
  }

  async function startRecording() {
    if (busy || recording) return;
    if (active && ["transcribing", "awaiting_review", "generating"].includes(active.status)) {
      setError("Finish the current report in the input bar before starting another one.");
      return;
    }
    setError("");
    const permission = await requestRecordingPermissionsAsync();
    if (!permission.granted) { setError("Microphone permission is required. Enable it in the phone settings."); return; }
    try {
      await setAudioModeAsync({ allowsRecording: true, playsInSilentMode: true });
      chunksRef.current = [];
      sampleRateRef.current = 16000;
      await audio.stream.start();
      setRecordingSeconds(0);
      recordingRef.current = true;
      setRecording(true);
    } catch (e) { setError(e instanceof Error ? e.message : "Could not start recording."); }
  }

  async function stopRecording() {
    if (!recording) return;
    audio.stream.stop();
    await new Promise((resolve) => setTimeout(resolve, 150));
    recordingRef.current = false;
    setRecording(false);
    await setAudioModeAsync({ allowsRecording: false, playsInSilentMode: true });
    const chunks = chunksRef.current;
    if (!chunks.length) { setError("No microphone audio was captured. Please try again."); return; }
    const wav = pcm16ToWav(chunks, sampleRateRef.current, 1);
    // React Native Blob does not accept ArrayBuffer parts on all Expo Go versions.
    await runTurn(wav);
  }

  async function approve() {
    if (!active || active.status !== "awaiting_review" || !draft.trim()) return;
    if (!active.id || active.id.startsWith("pending-")) { setError("The server has not assigned this turn yet. Please wait a moment and try again."); return; }
    setBusy(true); setError("");
    try { await submitTurn(active.id, draft.trim()); }
    catch (e) {
      setBusy(false);
      const message = e instanceof Error ? e.message : "Could not approve question.";
      setError(message.toLowerCase().includes("inference") ? "Another report is still processing. Check the input bar and try again when it finishes." : message);
    }
  }

  async function stop() {
    abortRef.current?.abort();
    if (active) { try { await cancelTurn(active.id); } catch { /* already stopped locally */ } }
    setBusy(false);
  }

  async function recover() {
    if (!active) return;
    try { setActive(await getTurn(active.id)); }
    catch (e) { setError(e instanceof Error ? e.message : "Could not recover turn."); }
  }

  async function pickFile() {
    const result = await DocumentPicker.getDocumentAsync({ type: "*/*", multiple: true, copyToCacheDirectory: true });
    if (!result.canceled) setAttachments((current) => [...current, ...result.assets.map((item) => ({ uri: item.uri, name: item.name, mimeType: item.mimeType, size: item.size }))]);
  }

  const displayTurns = active ? [active] : [];
  return <SafeAreaView style={styles.safe}><KeyboardAvoidingView style={styles.screen} behavior={Platform.OS === "ios" ? "padding" : "height"} keyboardVerticalOffset={0}>
    <View style={styles.header}><View><Text style={styles.title}>Incident Reporting Agent</Text><Text style={styles.subtitle}>{readiness?.stt.ready ? `Transcription · ${readiness.stt.name}` : "Connecting to transcription model…"}</Text><Text style={styles.subtitle}>{readiness?.reply.ready ? `Reply · ${readiness.reply.name}` : "Connecting to reply model…"}</Text></View><View style={[styles.dot, { backgroundColor: readiness?.stt.ready && readiness?.reply.ready ? colors.accent : "#d0d5dd" }]} /></View>
    <FlatList data={displayTurns} keyExtractor={(item) => item.id} contentContainerStyle={styles.messages} renderItem={({ item }) => { const question = item.approvedText; return <View style={styles.turn}>{question ? <View style={[styles.bubble, styles.userBubble]}><Text style={styles.label}>You</Text><Text style={styles.body}>{question}</Text></View> : null}{item.status === "transcribing" ? <View style={[styles.bubble, styles.assistantBubble, styles.statusBubble]}><ActivityIndicator color={colors.accent} /><Text style={styles.statusText}>Processing report…</Text></View> : null}{item.status === "generating" ? <View style={[styles.bubble, styles.assistantBubble]}><Text style={styles.label}>Incident agent</Text>{item.reply ? <Text style={styles.body}>{item.reply}</Text> : <Text style={styles.typing}>{typingDots}</Text>}{item.reply ? <Pressable onPress={() => void speakReply(item.reply)}><Text style={styles.action}>🔊 Speak</Text></Pressable> : null}</View> : null}{item.status === "completed" && item.reply ? <View style={[styles.bubble, styles.assistantBubble]}><Text style={styles.label}>Incident agent</Text><Text style={styles.body}>{item.reply}</Text><Pressable onPress={() => void speakReply(item.reply)}><Text style={styles.action}>🔊 Speak</Text></Pressable></View> : null}</View>; }} ListEmptyComponent={<View style={styles.empty}><Pressable onPress={recording ? stopRecording : startRecording} style={[styles.micButton, recording && styles.micButtonRecording]} disabled={busy && !recording}><MaterialIcons name={recording ? "stop" : "mic"} size={48} color="#fff" /></Pressable></View>} />
    {recording ? <View style={styles.recording}><Text style={styles.recordingText}>● Listening… {recordingSeconds}s</Text></View> : null}
    {attachments.length ? <View style={styles.chips}>{attachments.map((file, index) => <Pressable key={`${file.uri}-${index}`} onPress={() => setAttachments((items) => items.filter((_, i) => i !== index))} style={styles.chip}><Text numberOfLines={1} style={styles.chipText}>📎 {file.name}  ×</Text></Pressable>)}</View> : null}
    {error ? <View style={styles.error}><Text style={styles.errorText}>{error}</Text>{active ? <Pressable onPress={recover}><Text style={styles.action}>Read status</Text></Pressable> : null}</View> : null}
    {busy && active?.status === "transcribing" ? <View style={styles.processing}><ActivityIndicator color={colors.accent} /><Text style={styles.statusText}>Processing your report…</Text></View> : null}
    <Text style={styles.footnote}>{recording ? "Speak clearly, then tap the button again to review the transcription." : active?.status === "awaiting_review" ? "Check the input bar, edit the transcript, then tap ↑ to send." : active?.status === "generating" ? "Incident agent is responding…" : "Start an incident report · Ask by typing or speaking"}</Text>
    {!active || active.status === "awaiting_review" || active.status === "completed" || active.status === "failed" || active.status === "cancelled" ? <View style={styles.composer}><Pressable onPress={pickFile} style={styles.icon}><Text>＋</Text></Pressable><TextInput value={draft} onChangeText={setDraft} placeholder={active?.status === "awaiting_review" ? "Edit the transcript, then tap ↑ to send…" : "Or type an incident question…"} placeholderTextColor="#98a2b3" multiline style={styles.input} editable={!busy || active?.status === "awaiting_review"} /><Pressable onPress={active?.status === "awaiting_review" ? approve : busy ? stop : send} style={[styles.send, !draft.trim() && !busy && styles.disabled]}><MaterialIcons name={busy ? "stop" : "arrow-upward"} size={22} color="#fff" /></Pressable></View> : null}
  </KeyboardAvoidingView></SafeAreaView>;
}

const styles = StyleSheet.create({ safe: { flex: 1, backgroundColor: "#fff" }, screen: { flex: 1 }, header: { padding: 16, borderBottomWidth: 1, borderBottomColor: colors.line, flexDirection: "row", justifyContent: "space-between", alignItems: "center" }, title: { fontSize: 21, fontWeight: "700", color: colors.ink }, subtitle: { fontSize: 12, color: colors.muted, marginTop: 4 }, dot: { width: 10, height: 10, borderRadius: 5 }, messages: { padding: 16, gap: 14, flexGrow: 1 }, turn: { gap: 8 }, bubble: { borderRadius: 18, padding: 14, maxWidth: "90%" }, userBubble: { alignSelf: "flex-end", backgroundColor: colors.user }, assistantBubble: { alignSelf: "flex-start", backgroundColor: colors.panel }, label: { fontSize: 11, fontWeight: "700", color: colors.muted, marginBottom: 5 }, body: { fontSize: 16, lineHeight: 23, color: colors.ink }, action: { color: colors.accent, fontWeight: "700", marginTop: 10 }, empty: { alignItems: "center", justifyContent: "center", flex: 1, minHeight: 260 }, emptyTitle: { fontSize: 18, fontWeight: "700", color: colors.ink, marginBottom: 6 }, recording: { margin: 12, padding: 14, borderRadius: 16, backgroundColor: "#fff4ed", gap: 10 }, recordingText: { color: "#b54708", fontWeight: "700", textAlign: "center" }, processing: { flexDirection: "row", alignItems: "center", justifyContent: "center", gap: 8, paddingVertical: 6 }, statusBubble: { flexDirection: "row", alignItems: "center", gap: 8 }, statusText: { color: colors.muted, fontSize: 13 }, typing: { color: colors.muted, fontSize: 26, letterSpacing: 3, lineHeight: 30 }, review: { margin: 12, padding: 14, borderRadius: 16, borderWidth: 1, borderColor: "#b7dfd4", backgroundColor: "#f4fbf9" }, reviewInput: { minHeight: 70, maxHeight: 130, fontSize: 16, color: colors.ink, paddingVertical: 8 }, primary: { backgroundColor: colors.accent, padding: 13, alignItems: "center", borderRadius: 12 }, primaryText: { color: "#fff", fontWeight: "700" }, chips: { flexDirection: "row", flexWrap: "wrap", gap: 8, paddingHorizontal: 12 }, chip: { backgroundColor: colors.panel, borderRadius: 20, paddingHorizontal: 10, paddingVertical: 8, maxWidth: "90%" }, chipText: { color: colors.ink, fontSize: 12 }, error: { margin: 12, padding: 10, borderRadius: 10, backgroundColor: "#fff4ed" }, errorText: { color: "#b54708", fontSize: 13 }, micButton: { alignSelf: "center", width: 150, height: 150, borderRadius: 75, backgroundColor: colors.accent, alignItems: "center", justifyContent: "center", marginVertical: 8, shadowColor: colors.accent, shadowOpacity: 0.2, shadowRadius: 10, elevation: 4 }, micButtonRecording: { backgroundColor: "#b54708" }, composer: { margin: 12, borderWidth: 1, borderColor: colors.line, borderRadius: 20, minHeight: 54, flexDirection: "row", alignItems: "center", padding: 7, gap: 6 }, icon: { width: 38, height: 38, borderRadius: 19, backgroundColor: colors.panel, alignItems: "center", justifyContent: "center" }, input: { flex: 1, maxHeight: 110, fontSize: 16, color: colors.ink, paddingHorizontal: 6, paddingVertical: 9 }, send: { width: 38, height: 38, borderRadius: 19, backgroundColor: colors.accent, alignItems: "center", justifyContent: "center" }, disabled: { opacity: 0.4 }, footnote: { fontSize: 13, color: colors.muted, textAlign: "center", marginTop: 2, marginBottom: 2, paddingHorizontal: 12 } });
