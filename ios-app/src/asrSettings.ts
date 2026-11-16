/** Keep the user's spelling and phrase order while removing empty comma entries. */
export function parseVocabularyHints(value: string): string[] {
  return value.split(',').map(hint => hint.trim()).filter(hint => hint.length > 0);
}
