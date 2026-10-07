#!/usr/bin/env bash
# Deterministic tiny subprocess fixture: it never loads weights or uses network.
set -euo pipefail
printf '%s' "$$" > "$0.pid"
IFS= read -r init
printf '%s\n' "$init" >> "$0.log"
if [[ "$init" == *INIT_FAIL* ]]; then
    printf '%s\n' '{"type":"startup_error","message":"fixture initialization failed"}'
    exit 1
fi
if [[ "$init" == *INIT_HANG* ]]; then
    while IFS= read -r _; do :; done
    exit 0
fi
printf '%s\n' '{"type":"ready","protocol":1,"name":"fake-resident-model","load_time_ms":7}'
while IFS= read -r command; do
    printf '%s\n' "$command" >> "$0.log"
    case "$command" in
        *'"type":"shutdown"'*) exit 0 ;;
        *'"type":"generate"'*)
            [[ "$command" =~ \"id\":([0-9]+) ]]
            id="${BASH_REMATCH[1]}"
            case "$command" in
                *CRASH*) exit 7 ;;
                *STALE*) printf '%s\n' '{"type":"delta","id":999,"text":"stale"}' ;;
                *TRUNCATED*) printf '%s' '{"type":"done"'; exit 0 ;;
                *OVERSIZE*) printf '%1048577s\n' x ;;
                *MISMATCH*)
                    printf '{"type":"delta","id":%s,"text":"a"}\n' "$id"
                    printf '{"type":"done","id":%s,"text":"b"}\n' "$id"
                    ;;
                *CONTEXT_ERROR*) printf '{"type":"error","id":%s,"error":{"kind":"context_exceeded"}}\n' "$id" ;;
                *HANG*)
                    printf '{"type":"delta","id":%s,"text":"waiting"}\n' "$id"
                    while IFS= read -r _; do :; done
                    exit 0
                    ;;
                *WAIT*)
                    printf '{"type":"delta","id":%s,"text":"partial 世界"}\n' "$id"
                    IFS= read -r cancel
                    printf '%s\n' "$cancel" >> "$0.log"
                    [[ "$cancel" == *'"type":"cancel"'* ]]
                    printf '{"type":"delta","id":%s,"text":" late delta"}\n' "$id"
                    printf '{"type":"error","id":%s,"error":{"kind":"cancelled"}}\n' "$id"
                    ;;
                *)
                    printf '{"type":"delta","id":%s,"text":"Hello café "}\n' "$id"
                    # Deliberately split a UTF-8 character across stdout writes.
                    printf '{"type":"delta","id":%s,"text":"\344' "$id"
                    printf '\270\226\347\225\214 🧯"}\n'
                    printf '{"type":"done","id":%s,"text":"Hello café 世界 🧯"}\n' "$id"
                    ;;
            esac
            ;;
        *) exit 91 ;;
    esac
done
