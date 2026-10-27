#ifndef PHEME_VA_H
#define PHEME_VA_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct PhemeVaHandle PhemeVaHandle;
typedef struct PhemeVaObserver PhemeVaObserver;

enum PhemeVaCode {
    PHEME_VA_OK = 0,
    PHEME_VA_INVALID_ARGUMENT = 1,
    PHEME_VA_BACKEND_ERROR = 2,
    PHEME_VA_BUSY = 3,
    PHEME_VA_UNAVAILABLE = 4,
    PHEME_VA_UNSUPPORTED = 5,
    PHEME_VA_CLOSED = 6,
    PHEME_VA_PANIC = 7
};

/*
 * ABI/schema v1. All v1 symbols exist with and without the whisper feature.
 * Configuration JSON: schema_version=1, model_path (required);
 * language=null, dictionary=[], threads=4, max_audio_seconds=120,
 * metrics_enabled=true (defaults). Unknown fields are rejected.
 * Bounds: 16 KiB JSON, 4096-byte path, threads 1..16, duration 1..120,
 * 128 dictionary terms of at most 128 characters, total prompt <=2000.
 *
 * Every char** must be writable; it is set to NULL before validation.
 * On success it owns UTF-8 JSON; on failure it may own error JSON.
 * Release ANY non-null output with pheme_va_string_free, including errors.
 * JSON errors contain schema_version and error {code,message,run_id}.
 * Output strings are limited to 4 MiB. Null output is invalid.
 *
 * Calls may overlap on a live handle. Configure/inference reject overlap
 * with BUSY; inspect/observer calls do not wait for model/inference work.
 * Call synchronous operations away from UI/audio callbacks.
 * close stops admission; BUSY means outstanding work must settle. It does
 * not cancel inference. The host MUST join ALL calls before free, including
 * inspection/close. UI cancellation does not end native work or permit free.
 * Recoverable Rust panics become PHEME_VA_PANIC. Invalid/dangling pointers,
 * allocation aborts and native C/C++ failures cannot be contained by Rust.
 */
uint32_t pheme_va_abi_version(void);
bool pheme_va_has_whisper(void);
PhemeVaHandle *pheme_va_create(void);
int32_t pheme_va_configure(PhemeVaHandle *, const uint8_t *json, size_t length, char **output);
int32_t pheme_va_inspect(PhemeVaHandle *, char **output);

/*
 * Interleaved, finite f32 audio; 8–192 kHz, 1–8 channels, complete frames,
 * at most 16,777,216 samples and the configured duration. Size checks occur
 * before copying samples. A valid admitted operation has a generated run_id,
 * also returned in result JSON and used in its metric events.
 * Input buffers remain readable and immutable until the call returns.
 */
int32_t pheme_va_transcribe_v1(PhemeVaHandle *, const float *samples,
    size_t count, uint32_t sample_rate, uint16_t channels, char **output);
int32_t pheme_va_close(PhemeVaHandle *, char **output);
void pheme_va_free(PhemeVaHandle *);

/*
 * Observer owns a reference to the shared queue and may outlive the engine.
 * It has a separate lifetime: never free it while observer calls are active.
 * Exactly one host consumer should drain and fan out the batches to displays/
 * exporters. Multiple observers and the legacy drain compete for this queue.
 * Drain returns the existing MetricBatch[] schema; repeated drain returns [].
 * Inspect returns schema_version, capacity_events, pending_events, and
 * cumulative dropped_events. Overflow drops the oldest retained events.
 * Retry an exported batch using its original content and identity.
 */
PhemeVaObserver *pheme_va_observer_new(PhemeVaHandle *);
int32_t pheme_va_observer_drain(PhemeVaObserver *, char **output);
int32_t pheme_va_observer_inspect(PhemeVaObserver *, char **output);
void pheme_va_observer_free(PhemeVaObserver *);
void pheme_va_string_free(char *);

/* Legacy symbols below exist ONLY when built with the whisper feature.
 * Original signatures/0,1,2 codes are retained; busy/closed/panic use codes
 * above. Legacy transcribe returns plain text rather than structured JSON.
 * Null dictionary clears hints. Collection changes affect future runs.
 */
PhemeVaHandle *pheme_va_whisper_new(const char *model_path);
int32_t pheme_va_whisper_set_dictionary(PhemeVaHandle *, const char *dictionary);
int32_t pheme_va_transcribe(PhemeVaHandle *, const float *samples, size_t count,
    uint32_t sample_rate, uint16_t channels, char **output_text);
int32_t pheme_va_metrics_set_enabled(PhemeVaHandle *, bool enabled);
int32_t pheme_va_metrics_drain(PhemeVaHandle *, char **output_json);
void pheme_va_whisper_free(PhemeVaHandle *);

#ifdef __cplusplus
}
#endif
#endif
