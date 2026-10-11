pub fn clean_summary(input: &str) -> String {
    truncate_chars(&clean_text(input), 400)
}

pub fn clean_content(input: &str) -> String {
    clean_text(input)
}

pub(crate) fn clean_text(input: &str) -> String {
    let without_tags = strip_html(input);
    let decoded = decode_entities(&without_tags);
    collapse_whitespace(&decoded)
}

fn strip_html(input: &str) -> String {
    let chars = input.chars().collect::<Vec<_>>();
    let mut output = String::with_capacity(input.len());
    let mut index = 0;

    while index < chars.len() {
        if chars[index] == '<' {
            if let Some(tag_end) = tag_end(&chars, index) {
                output.push(' ');
                index = tag_end + 1;
                continue;
            }
        }
        output.push(chars[index]);
        index += 1;
    }

    output
}

/// Returns the index of the `>` closing a real tag that starts at `start`.
/// Text such as `5 < 10` or an unterminated `<` is left alone so that
/// loosely escaped feed content is never silently swallowed.
fn tag_end(chars: &[char], start: usize) -> Option<usize> {
    let opener = *chars.get(start + 1)?;
    if !(opener.is_ascii_alphabetic() || opener == '/' || opener == '!' || opener == '?') {
        return None;
    }
    chars[start + 1..]
        .iter()
        .position(|&ch| ch == '>')
        .map(|offset| start + 1 + offset)
}

fn decode_entities(input: &str) -> String {
    let chars = input.chars().collect::<Vec<_>>();
    let mut output = String::with_capacity(input.len());
    let mut index = 0;

    while index < chars.len() {
        if chars[index] != '&' {
            output.push(chars[index]);
            index += 1;
            continue;
        }

        match entity_at(&chars, index) {
            Some((decoded, next_index)) => {
                output.push(decoded);
                index = next_index;
            }
            None => {
                output.push('&');
                index += 1;
            }
        }
    }

    output
}

/// Decodes the entity starting at `start`, returning the character and the
/// index just past its `;`. A bare `&` (as in `AT&T` or `Q&A`) has no
/// terminator, so it is reported as "not an entity" and copied through
/// verbatim instead of consuming the text that follows it.
fn entity_at(chars: &[char], start: usize) -> Option<(char, usize)> {
    const MAX_ENTITY_LEN: usize = 12;

    let mut body = String::new();
    let mut index = start + 1;
    while index < chars.len() && body.chars().count() <= MAX_ENTITY_LEN {
        let ch = chars[index];
        if ch == ';' {
            return decode_entity_body(&body).map(|decoded| (decoded, index + 1));
        }
        if !(ch.is_ascii_alphanumeric() || ch == '#') {
            return None;
        }
        body.push(ch);
        index += 1;
    }

    None
}

fn decode_entity_body(body: &str) -> Option<char> {
    match body {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some(' '),
        _ => {
            let digits = body.strip_prefix('#')?;
            let value = match digits.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => digits.parse::<u32>().ok()?,
            };
            char::from_u32(value)
        }
    }
}

fn collapse_whitespace(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut last_was_space = false;

    for ch in input.chars() {
        if ch.is_whitespace() {
            if !last_was_space {
                output.push(' ');
            }
            last_was_space = true;
        } else {
            output.push(ch);
            last_was_space = false;
        }
    }

    output.trim().to_string()
}

fn truncate_chars(input: &str, max_chars: usize) -> String {
    let mut result: String = input.chars().take(max_chars).collect();
    if input.chars().count() > max_chars {
        result.push_str("...");
    }
    result
}

#[cfg(test)]
mod tests {
    use super::{clean_summary, clean_text};

    #[test]
    fn bare_ampersands_survive_entity_decoding() {
        assert_eq!(clean_text("AT&T raises prices"), "AT&T raises prices");
        assert_eq!(
            clean_text("Q&A tips and tricks for readers"),
            "Q&A tips and tricks for readers"
        );
        assert_eq!(clean_text("Ends with &amp"), "Ends with &amp");
        assert_eq!(clean_text("Rock &amp; Roll"), "Rock & Roll");
        assert_eq!(clean_text("&lt;tag&gt; &quot;x&quot;"), "<tag> \"x\"");
        assert_eq!(clean_text("&#39;a&#x27;b&#8217;"), "'a'b\u{2019}");
    }

    #[test]
    fn unescaped_angle_brackets_do_not_swallow_content() {
        assert_eq!(clean_text("5 < 10 is true"), "5 < 10 is true");
        assert_eq!(
            clean_text("An unterminated <div tag keeps its words"),
            "An unterminated <div tag keeps its words"
        );
        assert_eq!(clean_text("Profit > loss"), "Profit > loss");
        assert_eq!(clean_text("<p>Body <b>text</b></p>"), "Body text");
        assert_eq!(clean_text("<!-- note -->Visible"), "Visible");
    }

    #[test]
    fn summaries_are_cleaned_then_truncated() {
        let long = format!("<p>{}</p>", "word ".repeat(200));
        let summary = clean_summary(&long);
        assert_eq!(summary.chars().count(), 403);
        assert!(summary.ends_with("..."));
        assert_eq!(clean_summary("  spaced   out \n text "), "spaced out text");
    }
}
