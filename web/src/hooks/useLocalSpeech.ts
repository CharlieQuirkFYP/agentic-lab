import { useEffect, useRef, useState } from "react";

import { canAutoSpeak, localVoices } from "@/lib/voice/speech";

export function useLocalSpeech() {
  const supported =
    typeof window !== "undefined" && "speechSynthesis" in window;
  const [voices, setVoices] = useState<SpeechSynthesisVoice[]>([]);
  const [voiceURI, setVoiceURI] = useState("");
  const [autoSpeak, setAutoSpeak] = useState(true);
  const [speaking, setSpeaking] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const playback = useRef(0);
  const utterance = useRef<SpeechSynthesisUtterance | null>(null);
  const timeout = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);

  function stop() {
    playback.current++;
    clearTimeout(timeout.current);
    utterance.current = null;
    if (supported) window.speechSynthesis.cancel();
    setSpeaking(false);
  }

  useEffect(() => {
    if (!supported) return;
    const synthesis = window.speechSynthesis;
    const update = () => setVoices(localVoices(synthesis.getVoices()));
    update();
    synthesis.addEventListener("voiceschanged", update);
    const stopOnLeave = () => {
      playback.current++;
      clearTimeout(timeout.current);
      synthesis.cancel();
      setSpeaking(false);
    };
    window.addEventListener("pagehide", stopOnLeave);
    return () => {
      window.removeEventListener("pagehide", stopOnLeave);
      synthesis.removeEventListener("voiceschanged", update);
      stopOnLeave();
    };
  }, [supported]);

  function speak(text: string) {
    stop();
    setError(null);
    if (!text.trim()) return;
    if (!supported) {
      setError(
        "Speech playback is unavailable in this browser. The reply remains readable.",
      );
      return;
    }
    // Re-check execution location at playback time; never choose a default remote voice.
    const available = localVoices(window.speechSynthesis.getVoices());
    const selected =
      available.find((voice) => voice.voiceURI === voiceURI) ??
      available.find((voice) => voice.default) ??
      available[0];
    if (!selected) {
      setError(
        "No browser-reported local voice is installed. Install an OS voice or read the reply; remote voices are not used.",
      );
      return;
    }
    const token = playback.current;
    const message = new SpeechSynthesisUtterance(text);
    message.voice = selected;
    message.lang = selected.lang;
    utterance.current = message;
    message.onend = () => {
      if (token !== playback.current) return;
      clearTimeout(timeout.current);
      utterance.current = null;
      setSpeaking(false);
    };
    message.onerror = (event) => {
      if (token !== playback.current) return;
      stop();
      setError(
        `Local speech playback failed (${event.error}). The text is unchanged; use Replay to try again.`,
      );
    };
    try {
      setSpeaking(true);
      window.speechSynthesis.speak(message);
      timeout.current = setTimeout(() => {
        if (token !== playback.current) return;
        stop();
        setError(
          "Local speech playback timed out. The reply is still available as text.",
        );
      }, 180_000);
    } catch {
      stop();
      setError(
        "The browser could not start local speech playback. The text is unchanged.",
      );
    }
  }

  function liveReply(text: string) {
    if (canAutoSpeak(true, autoSpeak, text)) speak(text);
  }

  return {
    supported,
    voices,
    voiceURI,
    setVoiceURI,
    autoSpeak,
    setAutoSpeak,
    speaking,
    error,
    stop,
    speak,
    liveReply,
  };
}
