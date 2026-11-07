//! `robots.txt`: what a site asks automatic visitors to leave alone.

/// The rules that apply to Mohona.
#[derive(Clone, Debug, Default)]
pub struct Robots {
    rules: Vec<(bool, String)>,
    /// Seconds the site asks to wait between requests.
    pub crawl_delay: Option<f64>,
}

impl Robots {
    /// Reads a `robots.txt`: the group for `mohona` when there is one, else the group for `*`.
    pub fn parse(text: &str) -> Self {
        struct Group {
            agents: Vec<String>,
            rules: Vec<(bool, String)>,
            delay: Option<f64>,
        }
        let mut groups: Vec<Group> = Vec::new();
        let mut last_was_agent = false;
        for line in text.lines() {
            let line = line.split('#').next().unwrap_or("").trim();
            let Some((key, value)) = line.split_once(':') else {
                continue;
            };
            let (key, value) = (key.trim().to_ascii_lowercase(), value.trim());
            match key.as_str() {
                "user-agent" => {
                    if !last_was_agent || groups.is_empty() {
                        groups.push(Group {
                            agents: Vec::new(),
                            rules: Vec::new(),
                            delay: None,
                        });
                    }
                    groups
                        .last_mut()
                        .unwrap()
                        .agents
                        .push(value.to_ascii_lowercase());
                    last_was_agent = true;
                    continue;
                }
                "allow" | "disallow" => {
                    if let Some(g) = groups.last_mut() {
                        // An empty Disallow allows everything: no rule.
                        if !value.is_empty() {
                            g.rules.push((key == "allow", value.to_string()));
                        }
                    }
                }
                "crawl-delay" => {
                    if let (Some(g), Ok(d)) = (groups.last_mut(), value.parse::<f64>()) {
                        g.delay = Some(d);
                    }
                }
                _ => {}
            }
            last_was_agent = false;
        }
        let pick = groups
            .iter()
            .find(|g| g.agents.iter().any(|a| a.contains("mohona")))
            .or_else(|| groups.iter().find(|g| g.agents.iter().any(|a| a == "*")));
        match pick {
            Some(g) => Self {
                rules: g.rules.clone(),
                crawl_delay: g.delay.filter(|d| d.is_finite() && *d > 0.0),
            },
            None => Self::default(),
        }
    }

    /// Whether `path` (with its query) may be visited: the longest matching rule wins, and an
    /// allow wins a tie.
    pub fn allows(&self, path: &str) -> bool {
        let mut best: Option<(usize, bool)> = None;
        for (allow, pattern) in &self.rules {
            if matches(pattern, path) {
                let len = pattern.len();
                if best.is_none_or(|(l, a)| len > l || (len == l && *allow && !a)) {
                    best = Some((len, *allow));
                }
            }
        }
        best.is_none_or(|(_, allow)| allow)
    }
}

/// `*` matches anything, a final `$` the end; otherwise the pattern is a prefix.
fn matches(pattern: &str, path: &str) -> bool {
    let (pattern, anchored) = pattern
        .strip_suffix('$')
        .map_or((pattern, false), |p| (p, true));
    let parts: Vec<&str> = pattern.split('*').collect();
    let mut at = 0;
    for (n, part) in parts.iter().enumerate() {
        if n == 0 {
            if !path.starts_with(part) {
                return false;
            }
            at = part.len();
        } else if let Some(found) = path[at..].find(part) {
            at += found + part.len();
        } else {
            return false;
        }
    }
    !anchored || at == path.len() || parts.last().is_some_and(|l| l.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_longest_rule_wins_and_wildcards_work() {
        let r = Robots::parse(
            "User-agent: *\nDisallow: /private/\nAllow: /private/open/\nDisallow: /*.zip$\nCrawl-delay: 2\n",
        );
        assert!(r.allows("/index.html"));
        assert!(!r.allows("/private/a.html"));
        assert!(r.allows("/private/open/a.html"));
        assert!(!r.allows("/files/a.zip"));
        assert!(r.allows("/files/a.zip.txt"));
        assert_eq!(r.crawl_delay, Some(2.0));
    }

    #[test]
    fn a_group_for_mohona_beats_the_general_one() {
        let r =
            Robots::parse("User-agent: *\nDisallow: /\n\nUser-agent: Mohona\nDisallow: /secret\n");
        assert!(r.allows("/anything"));
        assert!(!r.allows("/secret/x"));
        let none = Robots::parse("User-agent: bot\nDisallow: /\n");
        assert!(none.allows("/x"));
        assert!(Robots::parse("User-agent: *\nDisallow:\n").allows("/x"));
    }
}
