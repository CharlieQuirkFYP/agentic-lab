import Pheme from '../modules/pheme-native/src/PhemeNativeModule';
import { saveLocalTranscript } from './localTranscriptStore';
import { createLocalChat } from './localChat';
import { Benchmark } from './phoneMetrics';

export const { getReadiness, startTurn, submitTurn, getTurn, cancelTurn, resetChat } =
  createLocalChat(Pheme, saveLocalTranscript, undefined, Benchmark);
