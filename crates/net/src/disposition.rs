use percent_encoding::percent_decode_str;

/// Extract the file name from a `Content-Disposition` header value.
///
/// Understands `filename="quoted"`, `filename=plain` and the encoded form
/// `filename*=UTF-8''%E2%82%AC.txt` (which wins when both are present).
/// The result is the raw name from the server: it may contain path separators
/// or other unsafe characters, so sanitize it before using it.
pub fn filename_from_content_disposition(value: &str) -> Option<String> {
    let mut plain = None;
    let mut extended = None;
    for part in split_parameters(value) {
        let Some((name, raw)) = part.split_once('=') else {
            continue;
        };
        let raw = raw.trim();
        match name.trim().to_ascii_lowercase().as_str() {
            "filename*" => extended = decode_extended(raw),
            "filename" => plain = Some(unquote(raw)),
            _ => {}
        }
    }
    extended.or(plain).filter(|name| !name.is_empty())
}

/// Split on `;`, ignoring semicolons inside quoted strings.
fn split_parameters(value: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut in_quotes = false;
    let mut escaped = false;
    let mut start = 0;
    for (index, character) in value.char_indices() {
        match character {
            _ if escaped => escaped = false,
            '\\' if in_quotes => escaped = true,
            '"' => in_quotes = !in_quotes,
            ';' if !in_quotes => {
                parts.push(&value[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    parts.push(&value[start..]);
    parts
}

fn unquote(raw: &str) -> String {
    match raw
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
    {
        // Only `\"` is treated as an escape. Other backslashes are kept,
        // because some servers send Windows paths, which sanitizing handles.
        Some(inner) => inner.replace("\\\"", "\""),
        None => raw.to_string(),
    }
}

/// Decode `charset'language'percent-encoded-text` (RFC 5987).
fn decode_extended(raw: &str) -> Option<String> {
    let mut pieces = raw.splitn(3, '\'');
    let charset = pieces.next()?;
    let _language = pieces.next()?;
    let encoded = pieces.next()?;
    let bytes: Vec<u8> = percent_decode_str(encoded).collect();
    if charset.is_empty() || charset.eq_ignore_ascii_case("utf-8") {
        Some(String::from_utf8_lossy(&bytes).into_owned())
    } else if charset.eq_ignore_ascii_case("iso-8859-1") {
        Some(bytes.iter().map(|&byte| byte as char).collect())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::filename_from_content_disposition as parse;

    #[test]
    fn quoted_and_plain_names() {
        assert_eq!(
            parse("attachment; filename=\"my file.zip\""),
            Some("my file.zip".into())
        );
        assert_eq!(
            parse("attachment; filename=report.pdf"),
            Some("report.pdf".into())
        );
        assert_eq!(parse("inline; FileName=\"A.txt\""), Some("A.txt".into()));
    }

    #[test]
    fn semicolon_inside_quotes_is_kept() {
        assert_eq!(
            parse("attachment; filename=\"a;b.txt\"; size=3"),
            Some("a;b.txt".into())
        );
    }

    #[test]
    fn escaped_quote_is_unescaped() {
        assert_eq!(
            parse("attachment; filename=\"say \\\"hi\\\".txt\""),
            Some("say \"hi\".txt".into())
        );
    }

    #[test]
    fn extended_name_wins_over_plain() {
        assert_eq!(
            parse("attachment; filename=\"fallback.txt\"; filename*=UTF-8''%E2%82%AC%20rates.txt"),
            Some("€ rates.txt".into())
        );
    }

    #[test]
    fn latin1_extended_name() {
        assert_eq!(
            parse("attachment; filename*=iso-8859-1'en'caf%E9.txt"),
            Some("café.txt".into())
        );
    }

    #[test]
    fn no_name_gives_none() {
        assert_eq!(parse("attachment"), None);
        assert_eq!(parse("attachment; filename=\"\""), None);
        assert_eq!(parse("attachment; filename*=weird'en'x"), None);
    }
}
