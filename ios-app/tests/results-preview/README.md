# Actual Results screen browser preview

Run from the repository root, using the already installed dependencies:

```sh
node web/node_modules/vite/bin/vite.js --config ios-app/tests/results-preview/vite.config.mjs
```

Open <http://127.0.0.1:5188/> at a 390 × 844 CSS-pixel viewport. The harness
imports `ios-app/app/metrics.tsx` and its real components and shared selectors.
Only the native storage hook, sync service, router, files and safe-area boundary
are replaced by Vite aliases. React, React DOM, React Native Web and SVG come
from the existing iOS dependencies. No fixture is imported into production.

The amber banner identifies all values as illustrative. They are deterministic
synthetic UI data, never phone benchmark evidence. The current conversation has
three turns, speech and reply operations, a cancelled third reply, sampled app
memory and CPU, categorical thermal state and unavailable GPU/power values.
Two saved-history rows and a separate sweep conversation support scope checks.

Preview routes:

- `/`: three-turn current conversation.
- `/?text=large`: the same screen at 135% text size. This uses actual text
  elements with unchanged viewport/chart geometry; native Dynamic Type still
  requires a device or simulator check.
- `/?conversation_id=illustrative-chat-saved`: saved two-turn conversation.
- `/?sweep_id=illustrative-sweep`: selected sweep scope.
- `/?fixture=zero`: valid zero memory readings.
- `/?fixture=single`: one valid memory reading plus explicit missing values.
- `/?fixture=missing`: all memory values explicitly unavailable.
- `/?fixture=none`: no memory readings retained.

Exercise the app's actual buttons: overview → App memory → a turn → a reply
operation → raw readings → back to all turns. The scoped total and chart must
change together. Verify history selection/search, metric selection and the
cancelled outcome. Check for horizontal overflow at 390px and with larger text.

Screenshots and validation summaries belong in the ignored directory
`ios-app/.expo/results-validation/`. Vite's cache is isolated there as well.

Verify the deterministic totals against the real selectors with
`node ios-app/tests/results-preview/validate-fixture.mjs` (Node 22.18+).

From a fresh standard-fixture overview, this browser-console command runs the
actual buttons through metric, turn, operation, raw readings, back, sharing and
filtered saved history. It asserts totals and horizontal bounds at each level:

```js
await (await import('/validate-browser.mjs')).runNavigation()
```

The validation helper captures `navigator.share` content locally. It does not
send or publish the report. Reload before each run. Use the same command at
`/?text=large` to repeat at 135% text size.
