import { requireOptionalNativeModule } from 'expo-modules-core';
import type { NativeTransport } from './PhemeNative.types';
import { createPhemeClient } from './client';

export default createPhemeClient(requireOptionalNativeModule<NativeTransport>('PhemeNative'));
