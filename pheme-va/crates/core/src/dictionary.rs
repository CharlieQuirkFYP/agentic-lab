use std::collections::HashSet;

use serde::Serialize;

#[derive(Clone, Debug, Default, Serialize)]
pub struct DictionaryHints {
    pub terms: Vec<String>,
    pub snippets: Vec<String>,
    pub max_prompt_characters: usize,
}

impl DictionaryHints {
    pub fn new() -> Self {
        Self {
            max_prompt_characters: 2_000,
            ..Self::default()
        }
    }

    pub fn with_terms<I, S>(terms: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut hints = Self::new();
        hints.terms = terms.into_iter().map(Into::into).collect();
        hints
    }

    pub fn prompt(&self) -> Option<DictionaryPrompt> {
        let mut values = Vec::new();
        let mut seen = HashSet::new();

        for value in self.terms.iter().chain(self.snippets.iter()) {
            let value = value.split_whitespace().collect::<Vec<_>>().join(" ");
            let value = value.trim();
            if value.is_empty() {
                continue;
            }
            let key = normalize(value);
            if key.is_empty() || !seen.insert(key) {
                continue;
            }
            values.push(value.to_owned());
        }

        let max = self.max_prompt_characters.max(1);
        let mut prompt = String::new();
        for value in values {
            let separator = if prompt.is_empty() { "" } else { ", " };
            if prompt.chars().count() + separator.chars().count() + value.chars().count() > max {
                break;
            }
            prompt.push_str(separator);
            prompt.push_str(&value);
        }

        (!prompt.is_empty()).then_some(DictionaryPrompt { text: prompt })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DictionaryPrompt {
    pub text: String,
}

fn normalize(value: &str) -> String {
    value
        .chars()
        .flat_map(char::to_lowercase)
        .map(|character| {
            if character.is_alphanumeric() || character.is_whitespace() {
                character
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_a_deduplicated_bounded_prompt() {
        let mut hints = DictionaryHints::with_terms([" KLASS ", "whisper.cpp", "KLASS"]);
        hints.snippets.push("west entrance".to_owned());
        hints.max_prompt_characters = 30;

        let prompt = hints.prompt().unwrap();
        assert_eq!(prompt.text, "KLASS, whisper.cpp");
        assert!(prompt.text.chars().count() <= 30);
    }

    #[test]
    fn ignores_empty_hints() {
        let hints = DictionaryHints::with_terms([" ", "\n"]);
        assert!(hints.prompt().is_none());
    }
}
