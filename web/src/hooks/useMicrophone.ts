import { useEffect, useLayoutEffect, useRef, useState } from "react";

import { errorMessage } from "@/lib/voice/api";
import {
  encodeWav,
  MAX_RECORDING_SECONDS,
  MAX_WAV_BYTES,
} from "@/lib/voice/wav";

interface Capture {
  context: AudioContext;
  stream?: MediaStream;
  source?: MediaStreamAudioSourceNode;
  node?: AudioWorkletNode;
  gain?: GainNode;
  chunks: Float32Array[];
  samples: number;
  finishing: boolean;
  timer?: ReturnType<typeof setInterval>;
  limitTimer?: ReturnType<typeof setTimeout>;
  acknowledge?: () => void;
}

function release(capture: Capture) {
  clearInterval(capture.timer);
  clearTimeout(capture.limitTimer);
  capture.stream?.getTracks().forEach((track) => {
    track.onended = null;
    track.stop();
  });
  if (capture.node) {
    capture.node.port.onmessage = null;
    capture.node.onprocessorerror = null;
    capture.node.port.close();
    capture.node.disconnect();
  }
  capture.source?.disconnect();
  capture.gain?.disconnect();
  void capture.context.close().catch(() => {});
  capture.chunks = [];
  capture.acknowledge?.();
}

export function useMicrophone(onClip: (clip: Blob) => void) {
  const [phase, setPhase] = useState<
    "idle" | "permission" | "recording" | "stopping"
  >("idle");
  const [seconds, setSeconds] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const active = useRef<Capture | null>(null);
  const onClipRef = useRef(onClip);
  useLayoutEffect(() => {
    onClipRef.current = onClip;
  }, [onClip]);

  function discard() {
    const capture = active.current;
    active.current = null;
    if (capture) release(capture);
    setPhase("idle");
    setSeconds(0);
  }

  async function finish() {
    const capture = active.current;
    if (!capture?.node || capture.finishing) return;
    capture.finishing = true;
    clearInterval(capture.timer);
    clearTimeout(capture.limitTimer);
    setPhase("stopping");
    try {
      await new Promise<void>((resolve, reject) => {
        const timeout = setTimeout(
          () =>
            reject(
              new Error("Audio capture did not finish. Please record again."),
            ),
          1500,
        );
        capture.acknowledge = () => {
          clearTimeout(timeout);
          resolve();
        };
        capture.node!.port.postMessage("stop");
      });
      if (active.current !== capture) return;
      const wav = encodeWav(capture.chunks, capture.context.sampleRate);
      discard();
      onClipRef.current(new Blob([wav], { type: "audio/wav" }));
    } catch (cause) {
      if (active.current !== capture) return;
      discard();
      setError(errorMessage(cause));
    }
  }

  async function start() {
    if (active.current) return;
    setError(null);
    if (!window.isSecureContext || !navigator.mediaDevices?.getUserMedia) {
      setError(
        "Microphone access requires localhost or HTTPS and a browser with audio capture support. You can still use text input.",
      );
      return;
    }
    if (!window.AudioContext || !window.AudioWorkletNode) {
      setError(
        "This browser does not support PCM audio capture. Please use text input or a newer browser.",
      );
      return;
    }
    let context: AudioContext;
    try {
      context = new AudioContext();
    } catch (cause) {
      setError(`Could not open audio capture: ${errorMessage(cause)}`);
      return;
    }
    const capture: Capture = {
      context,
      chunks: [],
      samples: 0,
      finishing: false,
    };
    active.current = capture;
    setSeconds(0);
    setPhase("permission");
    try {
      // Resume while still in the user gesture, before the permission prompt.
      await capture.context.resume();
      if (active.current !== capture) return;
      const stream = await navigator.mediaDevices.getUserMedia({
        audio: {
          channelCount: 1,
          echoCancellation: true,
          noiseSuppression: true,
        },
      });
      if (active.current !== capture) {
        stream.getTracks().forEach((track) => track.stop());
        return;
      }
      capture.stream = stream;
      await capture.context.audioWorklet.addModule(
        `${import.meta.env.BASE_URL}pcm-capture.worklet.js`,
      );
      if (active.current !== capture) return;
      capture.source = capture.context.createMediaStreamSource(stream);
      capture.node = new AudioWorkletNode(capture.context, "pcm-capture");
      capture.gain = capture.context.createGain();
      capture.gain.gain.value = 0;
      capture.node.port.onmessage = ({ data }) => {
        if (active.current !== capture) return;
        if (data.stopped) {
          capture.acknowledge?.();
          return;
        }
        if (!(data.samples instanceof Float32Array)) return;
        const maxSamples = Math.min(
          capture.context.sampleRate * MAX_RECORDING_SECONDS,
          Math.floor((MAX_WAV_BYTES - 44) / 2),
        );
        const remaining = maxSamples - capture.samples;
        if (remaining > 0) {
          const samples =
            data.samples.length > remaining
              ? data.samples.slice(0, remaining)
              : data.samples;
          capture.chunks.push(samples);
          capture.samples += samples.length;
        }
        if (capture.samples >= maxSamples) void finish();
      };
      capture.node.onprocessorerror = () => {
        if (active.current !== capture) return;
        discard();
        setError(
          "The audio capture processor failed. No recording was uploaded.",
        );
      };
      stream.getTracks().forEach((track) => {
        track.onended = () => {
          if (active.current !== capture) return;
          discard();
          setError(
            "The microphone disconnected. No incomplete recording was uploaded.",
          );
        };
      });
      capture.source
        .connect(capture.node)
        .connect(capture.gain)
        .connect(capture.context.destination);
      const started = performance.now();
      capture.timer = setInterval(
        () =>
          setSeconds(
            Math.min(
              MAX_RECORDING_SECONDS,
              Math.floor((performance.now() - started) / 1000),
            ),
          ),
        250,
      );
      capture.limitTimer = setTimeout(() => {
        void finish();
      }, MAX_RECORDING_SECONDS * 1000);
      setPhase("recording");
    } catch (cause) {
      if (active.current !== capture) return;
      discard();
      const name = cause instanceof Error ? cause.name : "";
      setError(
        name === "NotAllowedError"
          ? "Microphone permission was denied. Allow access in your browser settings, or use text input."
          : name === "NotFoundError"
            ? "No microphone was found. Connect a microphone or use text input."
            : errorMessage(cause),
      );
    }
  }

  useEffect(() => {
    const cleanup = () => {
      const capture = active.current;
      active.current = null;
      if (capture) release(capture);
    };
    const leavePage = () => {
      cleanup();
      setPhase("idle");
      setSeconds(0);
    };
    window.addEventListener("pagehide", leavePage);
    return () => {
      window.removeEventListener("pagehide", leavePage);
      cleanup();
    };
  }, []);

  return {
    phase,
    seconds,
    error,
    start,
    finish,
    discard,
    clearError: () => setError(null),
  };
}
