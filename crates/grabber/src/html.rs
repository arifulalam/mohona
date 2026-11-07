//! A forgiving reader of the links in a web page. No HTML parser library: only the few tags that
//! carry an address matter, and a page that is not well formed must still give what it can.

use url::Url;

/// How a page refers to an address.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A link (`<a href>`): a page or a file, decided by its ending.
    Anchor,
    /// A link marked `download`: a file whatever its ending.
    Download,
    /// Something shown in the page (picture, video, sound): a file.
    Media,
    /// A page inside the page (`<iframe>`): a page.
    Frame,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub url: Url,
    pub kind: Kind,
}

/// The links of `html`, resolved against `base` (the page's own address, or its `<base href>`),
/// in the order they appear. Only http and https addresses are kept, without fragments.
pub fn links(html: &str, base: &Url) -> Vec<Link> {
    let mut base = base.clone();
    let mut base_seen = false;
    let mut out = Vec::new();
    let bytes = html.as_bytes();
    let mut i = 0;
    while let Some(rel) = html[i..].find('<') {
        i += rel;
        let rest = &html[i..];
        if rest.starts_with("<!--") {
            i += rest.find("-->").map_or(rest.len(), |e| e + 3);
            continue;
        }
        let name_end = rest[1..]
            .find(|c: char| !c.is_ascii_alphanumeric())
            .map_or(rest.len(), |e| e + 1);
        let name = rest[1..name_end].to_ascii_lowercase();
        if name.is_empty() {
            i += 1;
            continue;
        }
        let (attrs, consumed, closed) = attributes(&rest[name_end..]);
        i += name_end + consumed;
        if !closed {
            break; // the page ends inside a tag: like a browser, drop it
        }
        if name == "script" || name == "style" {
            // Skip the content up to the closing tag.
            let lower = html[i.min(bytes.len())..].to_ascii_lowercase();
            let close = format!("</{name}");
            i += lower.find(&close).unwrap_or(lower.len());
            continue;
        }
        let get = |key: &str| {
            attrs
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.as_str())
        };
        let mut add = |value: &str, kind: Kind, base: &Url| {
            if let Some(url) = resolve(base, value) {
                out.push(Link { url, kind });
            }
        };
        match name.as_str() {
            "base" if !base_seen => {
                if let Some(url) = get("href").and_then(|h| resolve(&base, h)) {
                    base = url;
                    base_seen = true;
                }
            }
            "a" | "area" => {
                if let Some(href) = get("href") {
                    let kind = if get("download").is_some() {
                        Kind::Download
                    } else {
                        Kind::Anchor
                    };
                    add(href, kind, &base);
                }
            }
            "img" | "source" | "video" | "audio" | "track" | "embed" | "input" => {
                if name == "input" && get("type") != Some("image") {
                    continue;
                }
                for key in ["src", "data-src"] {
                    if let Some(v) = get(key) {
                        add(v, Kind::Media, &base);
                    }
                }
                for key in ["srcset", "data-srcset"] {
                    if let Some(v) = get(key) {
                        for candidate in v.split(',') {
                            if let Some(u) = candidate.split_whitespace().next() {
                                add(u, Kind::Media, &base);
                            }
                        }
                    }
                }
            }
            "iframe" | "frame" => {
                if let Some(v) = get("src") {
                    add(v, Kind::Frame, &base);
                }
            }
            _ => {}
        }
    }
    out
}

fn resolve(base: &Url, value: &str) -> Option<Url> {
    let value = value.trim();
    if value.is_empty() || value.starts_with('#') {
        return None;
    }
    let mut url = base.join(value).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    url.set_fragment(None);
    Some(url)
}

/// Reads the attributes of a tag (what follows its name) up to the closing `>`. Returns them
/// (names in lower case, values with character references decoded), how many bytes were used, and
/// whether the closing `>` was found.
fn attributes(text: &str) -> (Vec<(String, String)>, usize, bool) {
    let b = text.as_bytes();
    let mut i = 0;
    let mut out = Vec::new();
    while i < b.len() {
        match b[i] {
            b'>' => return (out, i + 1, true),
            c if c.is_ascii_whitespace() || c == b'/' => i += 1,
            _ => {
                let start = i;
                while i < b.len()
                    && !b[i].is_ascii_whitespace()
                    && !matches!(b[i], b'=' | b'>' | b'/')
                {
                    i += 1;
                }
                let name = text[start..i].to_ascii_lowercase();
                while i < b.len() && b[i].is_ascii_whitespace() {
                    i += 1;
                }
                let mut value = String::new();
                if i < b.len() && b[i] == b'=' {
                    i += 1;
                    while i < b.len() && b[i].is_ascii_whitespace() {
                        i += 1;
                    }
                    if i < b.len() && (b[i] == b'"' || b[i] == b'\'') {
                        let quote = b[i];
                        i += 1;
                        let from = i;
                        while i < b.len() && b[i] != quote {
                            i += 1;
                        }
                        value = decode(&text[from..i]);
                        i = (i + 1).min(b.len());
                    } else {
                        let from = i;
                        while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'>' {
                            i += 1;
                        }
                        value = decode(&text[from..i]);
                    }
                }
                if !name.is_empty() {
                    out.push((name, value));
                }
            }
        }
    }
    (out, b.len(), false)
}

/// `&amp;` and the other references that turn up in addresses.
fn decode(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let Some(end) = rest.find(';').filter(|e| *e <= 10) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let name = &rest[1..end];
        let decoded = match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => name
                .strip_prefix('#')
                .and_then(|n| {
                    n.strip_prefix(['x', 'X'])
                        .map_or_else(|| n.parse().ok(), |h| u32::from_str_radix(h, 16).ok())
                })
                .and_then(char::from_u32),
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(html: &str) -> Vec<(String, Kind)> {
        links(
            html,
            &Url::parse("https://example.com/dir/page.html").unwrap(),
        )
        .into_iter()
        .map(|l| (l.url.to_string(), l.kind))
        .collect()
    }

    #[test]
    fn links_are_resolved_and_classified() {
        let got = found(
            r##"<html><body>
            <a href="a.pdf">A</a> <A HREF='/b.zip?x=1&amp;y=2'>B</A>
            <a href=c.html>C</a> <a href="#top">top</a> <a href="mailto:x@y.z">m</a>
            <a href="javascript:void(0)">j</a> <a href="//cdn.example.org/d.png#frag">D</a>
            <a href="e.bin" download>E</a>
            <img src="img/p.jpg" srcset="img/p-2x.jpg 2x, img/p-3x.jpg 3x">
            <video src="v.mp4"><source src="v.webm"></video>
            <iframe src="/frame.html"></iframe>
            </body></html>"##,
        );
        let urls: Vec<&str> = got.iter().map(|(u, _)| u.as_str()).collect();
        assert_eq!(
            urls,
            vec![
                "https://example.com/dir/a.pdf",
                "https://example.com/b.zip?x=1&y=2",
                "https://example.com/dir/c.html",
                "https://cdn.example.org/d.png",
                "https://example.com/dir/e.bin",
                "https://example.com/dir/img/p.jpg",
                "https://example.com/dir/img/p-2x.jpg",
                "https://example.com/dir/img/p-3x.jpg",
                "https://example.com/dir/v.mp4",
                "https://example.com/dir/v.webm",
                "https://example.com/frame.html",
            ]
        );
        assert_eq!(got[4].1, Kind::Download);
        assert_eq!(got[5].1, Kind::Media);
        assert_eq!(got[10].1, Kind::Frame);
    }

    #[test]
    fn comments_scripts_and_a_base_tag_are_handled() {
        let got = found(
            r#"<base href="https://other.example/base/">
            <!-- <a href="hidden.zip">no</a> -->
            <script>var s = "<a href='in-script.zip'>";</script>
            <style>a { background: url(x.png) }</style>
            <a href="real.zip">yes</a><a href="broken"#,
        );
        assert_eq!(
            got.iter().map(|(u, _)| u.as_str()).collect::<Vec<_>>(),
            vec!["https://other.example/base/real.zip"]
        );
    }

    #[test]
    fn character_references_are_decoded() {
        assert_eq!(decode("a&amp;b&#38;c&#x26;d&nope;e&"), "a&b&c&d&nope;e&");
    }
}
