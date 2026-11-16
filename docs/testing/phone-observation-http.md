# Phone observation HTTP integration fixture

From the repository root, with Go and Node 22.18 or newer installed:

```sh
node ios-app/tests/benchmark-http-runner.mjs
```

The runner builds the Go API, binds `127.0.0.1:18081`, and creates its SQLite
database in a new operating-system temporary directory. It refuses an occupied
port, stops its own API when finished, and removes its temporary database and
binary. It overrides any inherited `API_ADDR` or `API_DATABASE_PATH` and does
not use the live phone collector. To reuse an API binary already built from
the current checkout:

```sh
PHEME_BENCHMARK_TEST_API_BINARY=/absolute/path/to/api-server \
  node ios-app/tests/benchmark-http-runner.mjs
```

The fixture uses the actual phone `createBenchmarkStore` with a Node SQLite
adapter, sends its pending immutable records through real HTTP, and compares
the API overview with the shared JSON, CSV, and Markdown report functions.
It covers 24 conversations with two turns each, transcription/reasoning
attempts, explicit terminal outcomes, model names and hashes, complete and
partial finalization, numeric zero, categorical state, and unavailable power.
More than 500 records exercise upload chunks and multiple 100-record pages.
It checks exact retries, three identity-conflict cases, atomic rejection,
late-arriving older revisions, filters, and an unchanged `as_of` snapshot
while a new conversation arrives.

`npm test` in `ios-app/` skips this HTTP test unless
`PHEME_BENCHMARK_TEST_URL` is explicitly supplied. Direct invocation is
available for an independently managed **empty disposable API/database**:

```sh
PHEME_BENCHMARK_TEST_URL=http://127.0.0.1:18081 \
  node --test ios-app/tests/benchmark-http.test.ts
```

The test rejects port 8080, non-loopback origins, redirects, and pre-existing
canonical observations, legacy metrics, or experiments before writing.
Installation, device, model, and metric fields explicitly identify synthetic
fixtures. Passing this test demonstrates persistence, HTTP, snapshot, and
reporting consistency only; it does not run models or establish physical
phone performance, sensor accuracy, power consumption, or battery endurance.
