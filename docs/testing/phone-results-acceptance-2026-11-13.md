# Phone Results validation — 2026-11-13

This record covers the Results implementation from the approved
[visual draft](../plans/phone-results-mockup/phone-results-draft.png). Test fixture
values are illustrative and are not phone measurements.

## Intended journey

1. Open **Results** from Chat. The selected current/latest manual conversation
   has processing-by-turn, app-memory and thermal graphs plus outcome totals.
2. Open **App memory**, inspect full-conversation statistics, then choose a turn
   and operation. Return to the same conversation overview.
3. Search/select saved conversations; open a benchmark through **View results**.
   Planned configurations that failed before starting a conversation stay visible.
4. Share a summary or JSON/CSV/Markdown from the displayed snapshot. Later
   observations must not change the already generated report.
5. Return to Chat while a reply is running. Reading results must not clear the
   active turn or reset model context.

## Automated and local checks

Validation is in progress. The final check results and any build/runtime
limitations will be recorded here after execution. Local diagnostic artifacts
belong in the ignored `ios-app/.expo/results-validation/` directory.

The focused checks cover scoped SQLite history and snapshots, ancestor and
finalization retention, derived processing totals, retries and unavailable
values, sample gaps and reduction, compatible comparisons, export snapshot
consistency, and foreground-only refresh scheduling. Run the complete relevant
suites from the following working directories:

```sh
# ios-app/
npm run typecheck
npm test
npm run test:native
npx expo export --platform ios --output-dir .expo/results-validation/export

# repository root
node --test shared/phone-benchmarks/tests/*.test.ts

# web/
npm test
npm run lint
npm run build
```

The added SVG renderer also requires CocoaPods integration and a native Expo
link check. Browser rendering of the actual React Native components validates
layout and interaction; it does not establish iPhone responsiveness or VoiceOver
behavior.

## Physical acceptance still required

The paired physical iPhone was reported **unavailable** by `xcrun devicectl list
devices` at the start of this implementation. Do not reuse the earlier collector
acceptance as evidence for these new screens.

- Complete a multi-turn manual conversation offline; inspect stage, memory and
  thermal details, relaunch, reopen the retained result and share it.
- Repeat with resource sampling disabled and with an interrupted turn. Missing
  samples remain unavailable; interrupted processing remains separately labelled.
- Test VoiceOver, larger system text and one-handed navigation on the phone.
- Inspect a mixed-success sweep and reconcile its displayed snapshot with all
  exports after uploads resume.
- Measure screen-open time, JS stalls, process memory and dropped observations
  with a near-capacity archive. Compare Results visible/hidden under repeated,
  counterbalanced fixed workloads before claiming a rendering overhead value.

GPU, watts and joules remain unsupported by the current measurement providers.
Execution completion is not incident correctness, saving or dispatch.
