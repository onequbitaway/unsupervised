//! Finds links in a text.

/// Characters that can stand around a link in a sentence. They are not a
/// part of the link.
const AROUND: &[char] =
    &['(', ')', '[', ']', '<', '>', '{', '}', '"', '\'', ',', '.', '!', '?', ';', ':'];

/// Returns the host of an address without its scheme: the text before the
/// path, the query, the fragment and the port, and behind the user name.
fn host_of(rest: &str) -> &str {
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..end];
    // `user:password@host`: the host is behind the last `@`.
    let host = authority.rsplit_once('@').map_or(authority, |(_, host)| host);
    host.split_once(':').map_or(host, |(host, _port)| host)
}

/// Returns true when `host` is a name with dots whose last part is one of
/// `endings`, for example `example.com`.
fn is_host_name(host: &str, endings: &[String]) -> bool {
    let mut labels = host.split('.');
    let Some(last) = labels.next_back() else {
        return false;
    };
    let mut others = 0;
    for label in labels {
        let good = !label.is_empty() && label.chars().all(|c| c.is_alphanumeric() || c == '-');
        if !good {
            return false;
        }
        others += 1;
    }
    others > 0 && endings.iter().any(|ending| ending.eq_ignore_ascii_case(last))
}

/// Returns the host of the link in one word, in lower case, when the word
/// is a link.
fn link_host(word: &str, endings: &[String]) -> Option<String> {
    let word = word.trim_matches(AROUND);
    let lower = word.to_lowercase();
    // `scheme://rest`, with a scheme of letters.
    if let Some((scheme, rest)) = lower.split_once("://")
        && !scheme.is_empty()
        && scheme.chars().all(|c| c.is_ascii_alphabetic())
        && !host_of(rest).is_empty()
    {
        return Some(host_of(rest).to_string());
    }
    let host = host_of(&lower);
    if lower.starts_with("www.") && host.len() > "www.".len() {
        return Some(host.to_string());
    }
    is_host_name(host, endings).then(|| host.to_string())
}

/// Returns the hosts of all links in a text, in lower case. A link is a
/// word with `://`, a word that starts with `www.`, or a host name whose
/// last part is in `endings`, with or without a path. A mail address
/// counts as a link to its host.
pub(crate) fn hosts(text: &str, endings: &[String]) -> Vec<String> {
    text.split_whitespace().filter_map(|word| link_host(word, endings)).collect()
}

/// Returns true when `host` is one of `listed` or a part of one of them:
/// `docs.example.org` is a part of `example.org`.
pub(crate) fn is_listed(host: &str, listed: &[String]) -> bool {
    listed.iter().any(|entry| {
        let entry = entry.to_lowercase();
        host == entry || host.strip_suffix(&entry).is_some_and(|start| start.ends_with('.'))
    })
}

#[cfg(test)]
mod tests {
    use super::{hosts, is_listed};

    fn endings() -> Vec<String> {
        ["com", "org", "ly"].map(String::from).to_vec()
    }

    #[test]
    fn links_are_found_with_and_without_a_scheme() {
        let cases = [
            ("see https://Example.com/a?b=c now", vec!["example.com"]),
            ("ftp://files.example.net:21/x", vec!["files.example.net"]),
            ("http://user:pw@evil.example/path", vec!["evil.example"]),
            ("go to www.Example.net.", vec!["www.example.net"]),
            ("(example.com), bit.ly/abc and more", vec!["example.com", "bit.ly"]),
            ("mail me: ada@example.org!", vec!["example.org"]),
            ("sub.domain.example.com/path", vec!["sub.domain.example.com"]),
        ];
        for (text, expected) in cases {
            assert_eq!(hosts(text, &endings()), expected, "{text}");
        }
    }

    #[test]
    fn plain_text_with_dots_is_not_a_link() {
        let texts = [
            "I work at the farm. Then I sleep.",
            "end.Then start",
            "e.g. bread, i.e. food",
            "version 1.2.3 of file.txt",
            "wait... what?",
            "a://",
            "www.",
            ".com",
            "price: 3.50",
            "no:// scheme",
        ];
        for text in texts {
            assert_eq!(hosts(text, &endings()), Vec::<String>::new(), "{text}");
        }
        // An ending that the list does not have.
        assert_eq!(hosts("example.net", &endings()), Vec::<String>::new());
    }

    #[test]
    fn a_listed_host_covers_its_parts_and_nothing_else() {
        let listed = vec!["Example.org".to_string()];
        assert!(is_listed("example.org", &listed));
        assert!(is_listed("docs.example.org", &listed));
        assert!(!is_listed("badexample.org", &listed));
        assert!(!is_listed("example.org.evil.com", &listed));
        assert!(!is_listed("example.com", &listed));
    }
}
