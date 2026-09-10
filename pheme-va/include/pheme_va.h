#ifndef PHEME_VA_H
#define PHEME_VA_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct PhemeVaHandle PhemeVaHandle;

bool pheme_va_has_whisper(void);

/* Available when the FFI library is built with the `whisper` feature. */
PhemeVaHandle *pheme_va_whisper_new(const char *model_path);

/* Set comma-separated dictionary terms. Returns 0 on success. */
int32_t pheme_va_whisper_set_dictionary(PhemeVaHandle *handle,
                                        const char *dictionary);

/*
 * `samples` contains interleaved f32 samples. The Rust core downmixes and
 * resamples them before sending them to Whisper. The returned string belongs to
 * Rust and must be released with pheme_va_string_free().
 *
 * Return codes: 0 success, 1 invalid arguments/audio, 2 inference failure.
 */
int32_t pheme_va_transcribe(PhemeVaHandle *handle, const float *samples,
                            size_t sample_count, uint32_t sample_rate,
                            uint16_t channels, char **output_text);

void pheme_va_whisper_free(PhemeVaHandle *handle);
void pheme_va_string_free(char *value);

#ifdef __cplusplus
}
#endif

#endif /* PHEME_VA_H */
