#ifndef PHEME_REPLY_H
#define PHEME_REPLY_H
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct ReplyHandle ReplyHandle;

/* All output strings belong to this framework, including errors.
 * Free once with pheme_reply_string_free, not the Whisper string allocator.
 * Join all outstanding calls before freeing the handle. */
ReplyHandle *pheme_reply_create(void);
int32_t pheme_reply_inspect(const ReplyHandle *, char **output);
int32_t pheme_reply_configure(const ReplyHandle *, const uint8_t *json, size_t length, char **output);
int32_t pheme_reply_respond(const ReplyHandle *, const uint8_t *json, size_t length, char **output);
int32_t pheme_reply_reset(const ReplyHandle *, char **output);
void pheme_reply_cancel(const ReplyHandle *);
void pheme_reply_free(ReplyHandle *);
void pheme_reply_string_free(char *);

#ifdef __cplusplus
}
#endif
#endif
