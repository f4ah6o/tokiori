//! Parsing of `TOKIORI_API_TOKENS` into bearer tokens.
//!
//! The variable holds comma-separated `name:scope` entries, for example
//! `ci:organization,assets:assets`. The `name` doubles as the secret the client
//! presents, and the `scope` is the one grant the token carries.

use crate::authorize::ApiToken;

/// Parses a raw `TOKIORI_API_TOKENS` value.
///
/// Entries are separated by `,`, and each entry is `name:scope` after trimming
/// whitespace around the entry and around each part. An entry without a colon,
/// or with an empty name or scope, is malformed and is skipped rather than
/// rejected, so one bad entry does not disable the others.
#[must_use]
pub fn parse_tokens(raw: &str) -> Vec<ApiToken> {
    raw.split(',')
        .filter_map(|entry| {
            let entry = entry.trim();
            let (name, scope) = entry.split_once(':')?;
            let (name, scope) = (name.trim(), scope.trim());
            if name.is_empty() || scope.is_empty() {
                return None;
            }
            Some(ApiToken {
                scope: scope.to_owned(),
                name: name.to_owned(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A token pair list that compares nicely in assertions.
    fn pairs(tokens: &[ApiToken]) -> Vec<(&str, &str)> {
        tokens
            .iter()
            .map(|token| (token.name.as_str(), token.scope.as_str()))
            .collect()
    }

    #[test]
    fn empty_string_yields_no_tokens() {
        assert_eq!(pairs(&parse_tokens("")), []);
    }

    #[test]
    fn whitespace_only_yields_no_tokens() {
        assert_eq!(pairs(&parse_tokens("  \t ")), []);
    }

    #[test]
    fn trims_whitespace_around_entry_and_parts() {
        assert_eq!(
            pairs(&parse_tokens("  ci : organization ")),
            vec![("ci", "organization")]
        );
    }

    #[test]
    fn skips_malformed_entries() {
        assert_eq!(
            pairs(&parse_tokens("no-colon,::,x:,:y,ok:scope")),
            vec![("ok", "scope")]
        );
    }

    #[test]
    fn parses_multiple_entries() {
        assert_eq!(
            pairs(&parse_tokens("ci:organization,assets:assets")),
            vec![("ci", "organization"), ("assets", "assets")]
        );
    }
}
