use std::fmt;
use std::str::FromStr;

use crate::wordlist_data::WORDS;

#[derive(Clone, PartialEq, Eq)]
pub struct Code(String);

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CodeError {
    #[error("a code looks like 7-guitarist-revenge: a number, a dash, then words")]
    Malformed,
    #[error("codes cannot contain spaces")]
    Spaces,
}

impl Code {
    #[must_use]
    pub fn nameplate(&self) -> &str {
        self.0.split('-').next().unwrap_or_default()
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub(crate) fn with_nameplate(nameplate: &str, words: &str) -> Self {
        Self(format!("{nameplate}-{words}"))
    }
}

impl FromStr for Code {
    type Err = CodeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.contains(char::is_whitespace) {
            return Err(CodeError::Spaces);
        }
        let Some((nameplate, words)) = s.split_once('-') else {
            return Err(CodeError::Malformed);
        };
        if !is_nameplate(nameplate) || words.split('-').any(str::is_empty) {
            return Err(CodeError::Malformed);
        }
        Ok(Self(s.to_owned()))
    }
}

impl fmt::Display for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Code(..)")
    }
}

#[must_use]
pub fn is_nameplate(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

#[must_use]
pub fn looks_like_code(s: &str) -> bool {
    s.parse::<Code>().is_ok_and(|code| {
        code.0
            .split('-')
            .skip(1)
            .all(|w| !w.is_empty() && w.bytes().all(|b| b.is_ascii_alphabetic()))
    })
}

#[must_use]
pub fn choose_words(count: usize) -> String {
    (0..count)
        .map(|i| {
            let (even, odd) = WORDS[usize::from(rand::random::<u8>())];
            if i % 2 == 0 { odd } else { even }
        })
        .collect::<Vec<_>>()
        .join("-")
}

#[must_use]
pub fn completions(prefix: &str, words: usize) -> Vec<String> {
    let typed_words = prefix.matches('-').count();
    if typed_words == 0 || typed_words > words {
        return Vec::new();
    }
    let (head, partial) = prefix.rsplit_once('-').unwrap_or(("", prefix));
    let odd_position = typed_words % 2 == 1;
    let mut out: Vec<String> = WORDS
        .iter()
        .map(|&(even, odd)| if odd_position { odd } else { even })
        .filter(|w| w.starts_with(partial))
        .map(|w| {
            let more = if typed_words < words { "-" } else { "" };
            format!("{head}-{w}{more}")
        })
        .collect();
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_codes_and_refuses_the_rest() {
        let code: Code = "7-guitarist-revenge".parse().unwrap();
        assert_eq!(code.nameplate(), "7");
        assert_eq!("7 guitarist".parse::<Code>(), Err(CodeError::Spaces));
        assert_eq!(
            "guitarist-revenge".parse::<Code>(),
            Err(CodeError::Malformed)
        );
        assert_eq!("7-".parse::<Code>(), Err(CodeError::Malformed));
        assert_eq!("7".parse::<Code>(), Err(CodeError::Malformed));
        assert_eq!("7--revenge".parse::<Code>(), Err(CodeError::Malformed));
        assert_eq!("7-guitarist-".parse::<Code>(), Err(CodeError::Malformed));
    }

    #[test]
    fn looks_like_code_rejects_paths() {
        assert!(looks_like_code("7-guitarist-revenge"));
        assert!(!looks_like_code("7-report.pdf"));
        assert!(!looks_like_code("photos/"));
    }

    #[test]
    fn chosen_words_alternate_odd_then_even() {
        let words = choose_words(3);
        let parts: Vec<_> = words.split('-').collect();
        assert_eq!(parts.len(), 3);
        assert!(WORDS.iter().any(|&(_, odd)| odd == parts[0]));
        assert!(WORDS.iter().any(|&(even, _)| even == parts[1]));
        assert!(WORDS.iter().any(|&(_, odd)| odd == parts[2]));
    }

    #[test]
    fn completes_the_word_in_progress() {
        assert_eq!(completions("7-guitari", 2), vec!["7-guitarist-".to_owned()]);
        assert_eq!(
            completions("7-guitarist-reven", 2),
            vec!["7-guitarist-revenge".to_owned()]
        );
        assert_eq!(completions("7", 2), Vec::<String>::new());
    }
}
