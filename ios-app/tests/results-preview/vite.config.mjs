import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.dirname(fileURLToPath(import.meta.url));
const ios = path.resolve(root, '../..');
const mock = name => path.join(root, name);

// Run using the existing web Vite installation. Nothing in this preview is part
// of the Expo application graph: the app screen itself is imported unchanged.
export default {
  root,
  server: { host: '127.0.0.1', port: 5188, strictPort: true, fs: { allow: [path.resolve(ios, '..')] } },
  cacheDir: path.join(ios, '.expo/results-validation/vite-cache'),
  resolve: {
    extensions: ['.web.tsx', '.web.ts', '.web.jsx', '.web.js', '.tsx', '.ts', '.jsx', '.js', '.json'],
    alias: [
      { find: /^.*\/usePhoneResults$/, replacement: mock('usePhoneResults.ts') },
      { find: /^.*\/phoneMetrics$/, replacement: mock('phoneMetrics.ts') },
      { find: 'expo-router', replacement: mock('router.tsx') },
      { find: 'expo-file-system', replacement: mock('files.ts') },
      { find: 'react-native-safe-area-context', replacement: mock('safeArea.tsx') },
      { find: /^react-native$/, replacement: path.join(ios, 'node_modules/react-native-web/dist/index.js') },
      { find: /^react-native-svg$/, replacement: path.join(ios, 'node_modules/react-native-svg/lib/module/elements.web.js') },
      { find: /^react-dom(\/.*)?$/, replacement: path.join(ios, 'node_modules/react-dom$1') },
      { find: /^react(\/.*)?$/, replacement: path.join(ios, 'node_modules/react$1') },
    ],
    dedupe: ['react', 'react-dom'],
  },
  esbuild: { jsx: 'automatic' },
  define: { __DEV__: true, 'process.env.NODE_ENV': JSON.stringify('development') },
};
