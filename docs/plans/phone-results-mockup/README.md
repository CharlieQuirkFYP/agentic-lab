# Phone Results visual draft

Open `index.html` locally, or view `phone-results-draft.png`. The SVG is the
scalable original. This is a static design, not the implemented app.

The three screens show the same illustrative conversation:

1. Overall metrics and graphs across its three turns.
2. App memory across the conversation, with sample statistics.
3. App memory while Turn 2's reply operation ran.

All numbers are example data. The generator computes sample statistics from its
illustrative readings. App memory is process-wide; narrowing to an operation
selects the readings taken during it, not memory attributable to that engine.
Hatched intervals have no readings. Phone content can scroll; only the first
two turn rows are shown in the middle screen.

Colours follow the current phone Metrics screen. Device proportions and safe
areas adapt the lark-apps creative-design iOS frame starter. The plan remains in
`../phone-results-graphs-plan.md`.

Run `node docs/plans/phone-results-mockup/draw.mjs` from the repository root to
regenerate the SVG and HTML. PNG rendering is a separate export step.
