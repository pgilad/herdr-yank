use crate::model::MatchSpan;
use regex::Regex;
use std::collections::HashMap;
use std::sync::OnceLock;

static BUILT_IN_PATTERNS: OnceLock<Vec<PatternDefinition>> = OnceLock::new();

/// Resource kinds that `kubectl` prints in front of `/name`, such as `pod/nginx-1`.
const KUBERNETES_RESOURCE_KINDS: &[&str] = &[
    "deployment.app",
    "binding",
    "componentstatuse",
    "configmap",
    "endpoint",
    "event",
    "limitrange",
    "namespace",
    "node",
    "persistentvolumeclaim",
    "persistentvolume",
    "pod",
    "podtemplate",
    "replicationcontroller",
    "resourcequota",
    "secret",
    "serviceaccount",
    "service",
    "mutatingwebhookconfiguration.admissionregistration.k8s.io",
    "validatingwebhookconfiguration.admissionregistration.k8s.io",
    "customresourcedefinition.apiextension.k8s.io",
    "apiservice.apiregistration.k8s.io",
    "controllerrevision.apps",
    "daemonset.apps",
    "deployment.apps",
    "replicaset.apps",
    "statefulset.apps",
    "tokenreview.authentication.k8s.io",
    "localsubjectaccessreview.authorization.k8s.io",
    "selfsubjectaccessreviews.authorization.k8s.io",
    "selfsubjectrulesreview.authorization.k8s.io",
    "subjectaccessreview.authorization.k8s.io",
    "horizontalpodautoscaler.autoscaling",
    "cronjob.batch",
    "job.batch",
    "certificatesigningrequest.certificates.k8s.io",
    "events.events.k8s.io",
    "daemonset.extensions",
    "deployment.extensions",
    "ingress.extensions",
    "networkpolicies.extensions",
    "podsecuritypolicies.extensions",
    "replicaset.extensions",
    "networkpolicie.networking.k8s.io",
    "poddisruptionbudget.policy",
    "clusterrolebinding.rbac.authorization.k8s.io",
    "clusterrole.rbac.authorization.k8s.io",
    "rolebinding.rbac.authorization.k8s.io",
    "role.rbac.authorization.k8s.io",
    "storageclasse.storage.k8s.io",
];

#[derive(Debug)]
struct PatternDefinition {
    name: String,
    /// Match precedence where lower numbers beat higher numbers.
    /// ie. `1` is the number one priority, while `5` is a lower priority
    priority: u16,
    regex: Regex,
    /// URL matches drop trailing punctuation and split at a Markdown `](` link boundary.
    is_url: bool,
}

impl PatternDefinition {
    fn compile(name: impl Into<String>, priority: u16, pattern: &str) -> Self {
        Self::try_compile(name, priority, pattern)
            .expect("built-in Herdr Yank pattern must compile")
    }

    fn try_compile(
        name: impl Into<String>,
        priority: u16,
        pattern: &str,
    ) -> Result<Self, regex::Error> {
        Ok(Self {
            name: name.into(),
            priority,
            regex: Regex::new(pattern)?,
            is_url: false,
        })
    }

    fn compile_url(pattern: &str) -> Self {
        let mut definition = Self::compile("url", 10, pattern);
        definition.is_url = true;
        definition
    }
}

/// User-defined regex pattern loaded from Herdr Yank config.
#[derive(Debug)]
pub struct CustomPatternDefinition(PatternDefinition);

impl CustomPatternDefinition {
    /// Compiles a user pattern, preserving regex diagnostics for config errors.
    pub fn compile(
        name: impl Into<String>,
        priority: u16,
        pattern: &str,
    ) -> Result<Self, regex::Error> {
        PatternDefinition::try_compile(name, priority, pattern).map(Self)
    }
}

/// Candidate match occurrence with its pattern definition order for tie-breaking.
#[derive(Debug, Clone, PartialEq, Eq)]
struct MatchCandidate {
    span: MatchSpan,
    pattern_order: usize,
}

/// Finds matches using user-defined custom patterns followed by the enabled built-ins.
///
/// `enabled_builtins` of `None` keeps every built-in pattern. A list keeps only the named
/// ones, which lets a config quiet down noisy patterns such as `number` or `path`.
pub fn find_matches(
    lines: &[String],
    custom_patterns: &[CustomPatternDefinition],
    enabled_builtins: Option<&[String]>,
) -> Vec<MatchSpan> {
    let mut patterns = Vec::with_capacity(custom_patterns.len() + built_in_patterns().len());
    patterns.extend(custom_patterns.iter().map(|pattern| &pattern.0));
    patterns.extend(
        built_in_patterns()
            .iter()
            .filter(|pattern| builtin_is_enabled(&pattern.name, enabled_builtins)),
    );
    find_matches_with_patterns(lines, &patterns)
}

/// Names of every built-in pattern, in match-priority order.
pub fn builtin_pattern_names() -> Vec<&'static str> {
    built_in_patterns()
        .iter()
        .map(|pattern| pattern.name.as_str())
        .collect()
}

fn builtin_is_enabled(name: &str, enabled_builtins: Option<&[String]>) -> bool {
    enabled_builtins.is_none_or(|names| names.iter().any(|enabled| enabled == name))
}

/// Finds what the `open-url` action can launch: web URLs and paths to local files.
///
/// Only the built-in `url` and `path` detectors run, so a custom pattern cannot change what a
/// hint opens. The launcher decides which targets qualify.
pub fn find_openable_targets(lines: &[String]) -> Vec<MatchSpan> {
    let patterns = [builtin_pattern("url"), builtin_pattern("path")];
    find_matches_with_patterns(lines, &patterns)
        .into_iter()
        .map(trim_path_end)
        .filter(|span| crate::url_opener::is_openable_target(&span.text))
        .collect()
}

/// Drops the trailing periods of a path match, so a sentence that ends with
/// `/tmp/report.html.` opens `/tmp/report.html`.
fn trim_path_end(mut span: MatchSpan) -> MatchSpan {
    if span.pattern == "path" {
        let len = span.text.trim_end_matches('.').len();
        span.text.truncate(len);
        span.end = span.start + len;
    }
    span
}

/// Looks a built-in up by name so that a reorder of the list cannot change the caller.
fn builtin_pattern(name: &str) -> &'static PatternDefinition {
    built_in_patterns()
        .iter()
        .find(|pattern| pattern.name == name)
        .expect("built-in Herdr Yank pattern must exist")
}

/// Builds the `kubernetes` regex from the literal resource kinds.
///
/// The `/name` part is required, so plain words such as `node` or `event` do not match. The
/// kind must not continue a word or a path, so `src/service/api.rs` stays one path.
fn kubernetes_pattern() -> String {
    let kinds = KUBERNETES_RESOURCE_KINDS
        .iter()
        .map(|kind| regex::escape(kind))
        .collect::<Vec<_>>()
        .join("|");
    format!(r"(?:^|[^\w/.~$@-])(?<match>(?:{kinds})/[[:alnum:]_#$%&+=/@.-]*[[:alnum:]])")
}

fn built_in_patterns() -> &'static [PatternDefinition] {
    BUILT_IN_PATTERNS.get_or_init(|| {
        vec![
            // Parentheses and brackets stay in the match, so `url_end` can keep balanced
            // pairs. Quotes, backticks, and angle brackets end a URL, as Markdown uses them.
            PatternDefinition::compile_url(
                r#"(?i:https?://|git@|git://|ssh://|ftp://|file://)[^\s"'`<>]+"#,
            ),
            PatternDefinition::compile("git-status", 15, r#"(modified|deleted|deleted by us|new file): +(?<match>.+)"#),
            PatternDefinition::compile(
                "git-status-branch",
                15,
                r#"Your branch is up to date with '(?<match>.*)'\."#,
            ),
            PatternDefinition::compile("diff", 15, r#"(---|\+\+\+) [ab]/(?<match>.*)"#),
            PatternDefinition::compile("kubernetes", 18, &kubernetes_pattern()),
            PatternDefinition::compile("path", 20, r#"(([.\w\-~\$@]+)?(/[.\w\-@]+)+/?)"#),
            PatternDefinition::compile(
                "uuid",
                30,
                r#"\b[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}\b"#,
            ),
            PatternDefinition::compile(
                "kubernetes-pod",
                35,
                r#"\b[a-z][a-z0-9-]*[a-z0-9]-[bcdfghjklmnpqrstvwxz2456789]{5,10}-[bcdfghjklmnpqrstvwxz2456789]{5}\b"#,
            ),
            // Upper bound covers SHA-512 hex so container digests and checksum output stay
            // selectable as one token instead of decaying into a numeric fragment.
            PatternDefinition::compile("git-sha", 40, r#"\b[0-9a-fA-F]{7,128}\b"#),
            PatternDefinition::compile("hex", 45, r#"\b0x[0-9a-fA-F]+\b"#),
            PatternDefinition::compile("ipv4", 50, r#"\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}"#),
            PatternDefinition::compile("number", 60, r#"[0-9]{4,}"#),
        ]
    })
}

fn find_matches_with_patterns(lines: &[String], patterns: &[&PatternDefinition]) -> Vec<MatchSpan> {
    let candidates = collect_candidates(lines, patterns);
    let mut accepted = resolve_overlaps(candidates);
    accepted.sort_by(|left, right| {
        left.line
            .cmp(&right.line)
            .then_with(|| left.start.cmp(&right.start))
            .then_with(|| left.end.cmp(&right.end))
    });
    accepted
}

/// Collects all candidate matches for all patterns with their pattern definition order for tie-breaking.
fn collect_candidates(lines: &[String], patterns: &[&PatternDefinition]) -> Vec<MatchCandidate> {
    let mut candidates = Vec::new();

    for (line_idx, line) in lines.iter().enumerate() {
        for (pattern_order, pattern) in patterns.iter().enumerate() {
            let mut search_from = 0;
            while search_from <= line.len() {
                let Some(captures) = pattern.regex.captures_at(line, search_from) else {
                    break;
                };
                let whole = captures
                    .get(0)
                    .expect("capture group 0 always participates");
                let regex_match = captures.name("match").unwrap_or(whole);
                search_from = next_search_start(line, whole.start(), whole.end());

                let start = regex_match.start();
                let mut end = regex_match.end();
                if pattern.is_url {
                    let link_end = markdown_link_end(line, start, end);
                    if link_end < end {
                        // In `[text](target)` the target after `](` is the next URL candidate.
                        search_from = link_end + 2;
                        end = link_end;
                    }
                    end = url_end(line, start, end);
                }
                if end == start {
                    continue;
                }

                candidates.push(MatchCandidate {
                    span: MatchSpan {
                        line: line_idx,
                        start,
                        end,
                        text: line[start..end].to_string(),
                        pattern: pattern.name.clone(),
                        priority: pattern.priority,
                    },
                    pattern_order,
                });
            }
        }
    }

    candidates
}

/// Returns where the next search starts. An empty match advances by one character.
fn next_search_start(line: &str, start: usize, end: usize) -> usize {
    if end > start {
        return end;
    }
    line[end..]
        .chars()
        .next()
        .map_or(line.len() + 1, |ch| end + ch.len_utf8())
}

/// Returns where a Markdown link boundary `](` cuts a URL match, or `end` without one.
fn markdown_link_end(line: &str, start: usize, end: usize) -> usize {
    line[start..end]
        .find("](")
        .map_or(end, |index| start + index)
}

/// Counts closing brackets that have no matching opener in a URL candidate.
#[derive(Debug, Default, Clone, Copy)]
struct UnbalancedClosers {
    paren: usize,
    bracket: usize,
    brace: usize,
}

impl UnbalancedClosers {
    fn count(text: &str) -> Self {
        let (mut opens, mut closers) = ([0usize; 3], [0usize; 3]);
        for ch in text.chars() {
            match ch {
                '(' => opens[0] += 1,
                ')' => closers[0] += 1,
                '[' => opens[1] += 1,
                ']' => closers[1] += 1,
                '{' => opens[2] += 1,
                '}' => closers[2] += 1,
                _ => {}
            }
        }
        Self {
            paren: closers[0].saturating_sub(opens[0]),
            bracket: closers[1].saturating_sub(opens[1]),
            brace: closers[2].saturating_sub(opens[2]),
        }
    }
}

/// Drops trailing punctuation, Markdown emphasis, and unmatched closing brackets from a URL.
///
/// Bracket balance is counted once and then decremented while trimming, so a long URL costs
/// one pass instead of a fresh scan per trimmed character.
fn url_end(line: &str, start: usize, end: usize) -> usize {
    let mut end = end;
    let mut unbalanced = UnbalancedClosers::count(&line[start..end]);

    while end > start {
        let Some(last) = line[start..end].chars().last() else {
            break;
        };
        let trim = match last {
            '.' | ',' | ';' | ':' | '!' | '?' | '*' | '_' => true,
            ')' => unbalanced.paren > 0,
            ']' => unbalanced.bracket > 0,
            '}' => unbalanced.brace > 0,
            _ => false,
        };
        if !trim {
            break;
        }

        match last {
            ')' => unbalanced.paren -= 1,
            ']' => unbalanced.bracket -= 1,
            '}' => unbalanced.brace -= 1,
            _ => {}
        }
        end -= last.len_utf8();
    }

    end
}

/// Resolves overlapping candidates by acceptance priority where lower numbers are higher priority.
///
/// Overlap is line-local, so accepted spans are bucketed per line. A busy pane then compares
/// each candidate against its own line instead of against every span already accepted.
fn resolve_overlaps(mut candidates: Vec<MatchCandidate>) -> Vec<MatchSpan> {
    candidates.sort_by(|left, right| {
        left.span
            .priority
            .cmp(&right.span.priority)
            .then_with(|| right.span.len_bytes().cmp(&left.span.len_bytes()))
            .then_with(|| left.span.line.cmp(&right.span.line))
            .then_with(|| left.span.start.cmp(&right.span.start))
            .then_with(|| left.span.end.cmp(&right.span.end))
            .then_with(|| left.pattern_order.cmp(&right.pattern_order))
    });

    let mut accepted_by_line: HashMap<usize, Vec<MatchSpan>> = HashMap::new();
    let mut accepted_count = 0;
    for candidate in candidates {
        let line_spans = accepted_by_line.entry(candidate.span.line).or_default();
        if line_spans
            .iter()
            .all(|accepted_span| !candidate.span.overlaps(accepted_span))
        {
            line_spans.push(candidate.span);
            accepted_count += 1;
        }
    }

    let mut accepted = Vec::with_capacity(accepted_count);
    for spans in accepted_by_line.into_values() {
        accepted.extend(spans);
    }
    accepted
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find_matches_with_defaults_only(lines: &[String]) -> Vec<MatchSpan> {
        let patterns = built_in_patterns().iter().collect::<Vec<_>>();
        find_matches_with_patterns(lines, &patterns)
    }

    fn lines<const N: usize>(values: [&str; N]) -> Vec<String> {
        values.into_iter().map(String::from).collect()
    }

    fn test_pattern(name: &'static str, priority: u16, pattern: &str) -> PatternDefinition {
        PatternDefinition::compile(name, priority, pattern)
    }

    fn test_patterns(patterns: &[PatternDefinition]) -> Vec<&PatternDefinition> {
        patterns.iter().collect()
    }

    #[test]
    fn finds_url_schemes() {
        let matches = find_matches_with_defaults_only(&lines([
            "https://example.com http://example.com git@github.com:org/repo.git",
            "git://host/repo ssh://host/path ftp://host/path file:///tmp/file",
        ]));
        let urls: Vec<&str> = matches
            .iter()
            .filter(|span| span.pattern == "url")
            .map(|span| span.text.as_str())
            .collect();

        assert_eq!(
            urls,
            vec![
                "https://example.com",
                "http://example.com",
                "git@github.com:org/repo.git",
                "git://host/repo",
                "ssh://host/path",
                "ftp://host/path",
                "file:///tmp/file",
            ]
        );
    }

    #[test]
    fn url_stops_at_delimiters() {
        let matches = find_matches_with_defaults_only(&lines([
            "(https://a.test/path) 'https://b.test' \"https://c.test\"",
        ]));
        let urls: Vec<&str> = matches
            .iter()
            .filter(|span| span.pattern == "url")
            .map(|span| span.text.as_str())
            .collect();

        assert_eq!(
            urls,
            vec!["https://a.test/path", "https://b.test", "https://c.test"]
        );
    }

    #[test]
    fn url_trims_sentence_punctuation() {
        let matches = find_matches_with_defaults_only(&lines([
            "see https://example.com/path., then https://example.com/what?!",
        ]));
        let urls = matches
            .iter()
            .filter(|span| span.pattern == "url")
            .map(|span| span.text.as_str())
            .collect::<Vec<_>>();

        assert_eq!(
            urls,
            vec!["https://example.com/path", "https://example.com/what"]
        );
    }

    #[test]
    fn openable_urls_are_only_http_and_https() {
        let matches = find_openable_targets(&lines([
            "HTTP://example.com HTTPS://secure.test file:///tmp/a file://server/share",
            "ftp://host/a git://host/repo ssh://host/repo git@github.com:org/repo.git /tmp/a",
        ]));
        let urls = matches
            .iter()
            .map(|span| span.text.as_str())
            .collect::<Vec<_>>();

        assert_eq!(urls, vec!["HTTP://example.com", "HTTPS://secure.test"]);
    }

    fn url_texts(line: &str) -> Vec<String> {
        find_matches_with_defaults_only(&lines([line]))
            .into_iter()
            .filter(|span| span.pattern == "url")
            .map(|span| span.text)
            .collect()
    }

    #[test]
    fn urls_in_markdown_do_not_take_the_markup() {
        assert_eq!(
            url_texts("see `https://example.com/a` and <https://example.com/b> or **https://example.com/c**"),
            [
                "https://example.com/a",
                "https://example.com/b",
                "https://example.com/c"
            ]
        );
        assert_eq!(
            url_texts("_https://example.com/d_."),
            ["https://example.com/d"]
        );
    }

    #[test]
    fn balanced_parentheses_stay_in_a_url() {
        assert_eq!(
            url_texts("read https://en.wikipedia.org/wiki/Rust_(programming_language)."),
            ["https://en.wikipedia.org/wiki/Rust_(programming_language)"]
        );
        assert_eq!(
            url_texts("(see https://example.com/x)."),
            ["https://example.com/x"]
        );
    }

    #[test]
    fn a_markdown_link_gives_its_text_url_and_its_target_url() {
        assert_eq!(
            url_texts("[https://example.com/text](https://example.com/target)"),
            ["https://example.com/text", "https://example.com/target"]
        );
        assert_eq!(
            url_texts("[docs](https://example.com/docs)"),
            ["https://example.com/docs"]
        );
    }

    #[test]
    fn empty_custom_matches_do_not_stop_the_search() {
        let patterns = vec![test_pattern("maybe", 10, r#"x*"#)];
        let matches = find_matches_with_patterns(&lines(["axxb x"]), &test_patterns(&patterns));
        let texts: Vec<&str> = matches.iter().map(|span| span.text.as_str()).collect();

        assert_eq!(texts, ["xx", "x"]);
    }

    #[test]
    fn openable_urls_do_not_use_custom_patterns() {
        let custom =
            vec![CustomPatternDefinition::compile("url", 1, r#"https://custom\.test"#).unwrap()];
        let all = find_matches(&lines(["https://custom.test"]), &custom, None);
        let openable = find_openable_targets(&lines(["https://custom.test"]));

        assert_eq!(all[0].priority, 1);
        assert_eq!(openable[0].priority, 10);
    }

    #[test]
    fn openable_targets_include_existing_local_files_of_an_allowed_type() {
        let dir = crate::test_support::TempDir::new("openable-files");
        std::fs::write(dir.join("report.html"), "<html></html>").unwrap();
        std::fs::write(dir.join("run.command"), "#!/bin/sh").unwrap();
        let dir = dir.to_str().unwrap();

        let matches = find_openable_targets(&lines([
            &format!("wrote {dir}/report.html."),
            &format!("`{dir}/run.command` {dir}/missing.html https://example.com/a.html"),
        ]));
        let texts = matches
            .iter()
            .map(|span| span.text.as_str())
            .collect::<Vec<_>>();

        let report = format!("{dir}/report.html");
        assert_eq!(texts, [report.as_str(), "https://example.com/a.html"]);
        assert_eq!(matches[0].end - matches[0].start, report.len());
    }

    #[test]
    fn finds_style_paths() {
        let matches =
            find_matches_with_defaults_only(&lines(["/tmp/foo/bar src/main.rs foo/bar dir/sub/"]));
        let paths: Vec<&str> = matches
            .iter()
            .filter(|span| span.pattern == "path")
            .map(|span| span.text.as_str())
            .collect();

        assert_eq!(
            paths,
            vec!["/tmp/foo/bar", "src/main.rs", "foo/bar", "dir/sub/"]
        );
    }

    #[test]
    fn finds_uppercase_and_lowercase_uuids() {
        let matches = find_matches_with_defaults_only(&lines([
            "ids 123e4567-e89b-12d3-a456-426614174000 123E4567-E89B-12D3-A456-426614174000",
        ]));
        let uuids: Vec<&str> = matches
            .iter()
            .filter(|span| span.pattern == "uuid")
            .map(|span| span.text.as_str())
            .collect();

        assert_eq!(
            uuids,
            vec![
                "123e4567-e89b-12d3-a456-426614174000",
                "123E4567-E89B-12D3-A456-426614174000",
            ]
        );
    }

    #[test]
    fn finds_git_sha_lengths_with_uppercase_support() {
        let matches = find_matches_with_defaults_only(&lines([
            "sha abcDEF1 abcdef1234567890abcdef1234567890abcdef12 abcdef1234567890abcdef1234567890abcdef123",
        ]));
        let shas: Vec<&str> = matches
            .iter()
            .filter(|span| span.pattern == "git-sha")
            .map(|span| span.text.as_str())
            .collect();

        assert_eq!(
            shas,
            vec![
                "abcDEF1",
                "abcdef1234567890abcdef1234567890abcdef12",
                "abcdef1234567890abcdef1234567890abcdef123",
            ]
        );
    }

    #[test]
    fn finds_whole_sha256_digests_instead_of_numeric_fragments() {
        let digest = "9f5f0a2c1b7e3d4a5b6c7d8e9f0a1b2c3d4e5f60718293a4b5c6d7e8f9a0b1c2";
        let matches = find_matches_with_defaults_only(&lines([&format!("docker sha256:{digest}")]));
        let texts: Vec<&str> = matches.iter().map(|span| span.text.as_str()).collect();

        assert!(texts.contains(&digest));
        assert!(!matches.iter().any(|span| span.pattern == "number"));
    }

    #[test]
    fn hex_string_longer_than_the_sha_bound_is_left_alone() {
        let too_long = "a".repeat(129);
        let matches = find_matches_with_defaults_only(&lines([&too_long]));

        assert!(!matches.iter().any(|span| span.pattern == "git-sha"));
    }

    #[test]
    fn finds_loose_ipv4() {
        let matches =
            find_matches_with_defaults_only(&lines(["hosts 192.168.1.10 999.999.999.999"]));
        let ips: Vec<&str> = matches
            .iter()
            .filter(|span| span.pattern == "ipv4")
            .map(|span| span.text.as_str())
            .collect();

        assert_eq!(ips, vec!["192.168.1.10", "999.999.999.999"]);
    }

    #[test]
    fn finds_long_numbers_inside_text() {
        let matches = find_matches_with_defaults_only(&lines(["zzz1234zzz issue-5678 foo_9012"]));
        let numbers: Vec<&str> = matches
            .iter()
            .filter(|span| span.pattern == "number")
            .map(|span| span.text.as_str())
            .collect();

        assert_eq!(numbers, vec!["1234", "5678", "9012"]);
    }

    #[test]
    fn finds_hex_literals_before_numeric_submatches() {
        let matches =
            find_matches_with_defaults_only(&lines(["values 0xdeadBEEF 0x1234 not0xabc"]));
        let hexes: Vec<&str> = matches
            .iter()
            .filter(|span| span.pattern == "hex")
            .map(|span| span.text.as_str())
            .collect();

        assert_eq!(hexes, vec!["0xdeadBEEF", "0x1234"]);
        assert!(!matches
            .iter()
            .any(|span| span.pattern == "number" && span.text == "1234"));
    }

    #[test]
    fn finds_kubernetes_resource_references() {
        let matches = find_matches_with_defaults_only(&lines([
            "kubectl get pod/nginx-123 service/api deployment.apps/frontend",
        ]));
        let resources: Vec<&str> = matches
            .iter()
            .filter(|span| span.pattern == "kubernetes")
            .map(|span| span.text.as_str())
            .collect();

        assert_eq!(
            resources,
            vec!["pod/nginx-123", "service/api", "deployment.apps/frontend"]
        );
    }

    #[test]
    fn kubernetes_needs_a_resource_name() {
        let matches = find_matches_with_defaults_only(&lines([
            "the node process emitted an event about the secret service in this namespace",
            "node_modules/react event_handler pod/ deploymentXapp/web",
        ]));

        assert!(
            !matches.iter().any(|span| span.pattern == "kubernetes"),
            "{matches:?}"
        );
    }

    #[test]
    fn kubernetes_does_not_take_a_path_segment() {
        let matches = find_matches_with_defaults_only(&lines([
            "edit src/service/api.rs and /var/lib/node/data",
        ]));
        let texts: Vec<(&str, &str)> = matches
            .iter()
            .map(|span| (span.pattern.as_str(), span.text.as_str()))
            .collect();

        assert_eq!(
            texts,
            [
                ("path", "src/service/api.rs"),
                ("path", "/var/lib/node/data")
            ]
        );
    }

    #[test]
    fn kubernetes_names_can_hold_dots() {
        let matches = find_matches_with_defaults_only(&lines([
            "configmap/kube-root-ca.crt created (pod/web-1).",
        ]));
        let resources: Vec<&str> = matches
            .iter()
            .filter(|span| span.pattern == "kubernetes")
            .map(|span| span.text.as_str())
            .collect();

        assert_eq!(resources, ["configmap/kube-root-ca.crt", "pod/web-1"]);
    }

    #[test]
    fn finds_deployment_managed_kubernetes_pod_names() {
        let matches = find_matches_with_defaults_only(&lines([
            "pods nginx-deployment-66b6c48dd5-7xb2r api-abcde-12345 bad-pod-abcde-aeiou",
        ]));
        let pods: Vec<&str> = matches
            .iter()
            .filter(|span| span.pattern == "kubernetes-pod")
            .map(|span| span.text.as_str())
            .collect();

        assert_eq!(pods, vec!["nginx-deployment-66b6c48dd5-7xb2r"]);
    }

    #[test]
    fn finds_git_status_paths_with_named_capture() {
        let matches = find_matches_with_defaults_only(&lines([
            "modified:   src/main.rs",
            "deleted by us: docs/old.md",
            "new file:   README.md",
        ]));
        let statuses: Vec<&MatchSpan> = matches
            .iter()
            .filter(|span| span.pattern == "git-status")
            .collect();

        assert_eq!(
            statuses
                .iter()
                .map(|span| span.text.as_str())
                .collect::<Vec<_>>(),
            vec!["src/main.rs", "docs/old.md", "README.md"]
        );
        assert_eq!(statuses[0].start, "modified:   ".len());
    }

    #[test]
    fn finds_git_status_branch_name_with_named_capture() {
        let matches = find_matches_with_defaults_only(&lines([
            "Your branch is up to date with 'origin/main'.",
        ]));
        let branch = matches
            .iter()
            .find(|span| span.pattern == "git-status-branch")
            .unwrap();

        assert_eq!(branch.text, "origin/main");
    }

    #[test]
    fn finds_diff_paths_with_named_capture() {
        let matches =
            find_matches_with_defaults_only(&lines(["--- a/src/lib.rs", "+++ b/src/lib.rs"]));
        let diff_paths: Vec<&str> = matches
            .iter()
            .filter(|span| span.pattern == "diff")
            .map(|span| span.text.as_str())
            .collect();

        assert_eq!(diff_paths, vec!["src/lib.rs", "src/lib.rs"]);
    }

    #[test]
    fn expanded_patterns_win_over_lower_priority_path_and_number_overlaps() {
        let matches =
            find_matches_with_defaults_only(&lines(["+++ b/src/lib.rs", "pod/nginx 0x1234"]));
        let patterns: Vec<&str> = matches.iter().map(|span| span.pattern.as_str()).collect();

        assert_eq!(patterns, vec!["diff", "kubernetes", "hex"]);
    }

    #[test]
    fn expanded_patterns_avoid_common_false_positive_boundaries() {
        let matches =
            find_matches_with_defaults_only(&lines(["not0xabc serviceable api-abcde-aeiou"]));
        let expanded: Vec<&MatchSpan> = matches
            .iter()
            .filter(|span| {
                matches!(
                    span.pattern.as_str(),
                    "hex" | "kubernetes" | "kubernetes-pod"
                )
            })
            .collect();

        assert!(expanded.is_empty());
    }

    #[test]
    fn named_match_capture_defines_text_and_range() {
        let patterns = vec![test_pattern("status", 10, r#"status=\[(?<match>[^\]]+)\]"#)];
        let matches =
            find_matches_with_patterns(&lines(["status=[copy-me]"]), &test_patterns(&patterns));

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].text, "copy-me");
        assert_eq!(matches[0].start, 8);
        assert_eq!(matches[0].end, 15);
    }

    #[test]
    fn full_match_is_used_without_named_capture() {
        let patterns = vec![test_pattern("word", 10, r#"copy-me"#)];
        let matches =
            find_matches_with_patterns(&lines(["xx copy-me yy"]), &test_patterns(&patterns));

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].text, "copy-me");
        assert_eq!(matches[0].start, 3);
        assert_eq!(matches[0].end, 10);
    }

    #[test]
    fn overlap_resolution_uses_named_capture_range() {
        let patterns = vec![
            test_pattern("wrapped", 10, r#"prefix-(?<match>abc)-suffix"#),
            test_pattern("prefix", 20, r#"prefix"#),
        ];
        let matches =
            find_matches_with_patterns(&lines(["prefix-abc-suffix"]), &test_patterns(&patterns));
        let texts: Vec<&str> = matches.iter().map(|span| span.text.as_str()).collect();

        assert_eq!(texts, vec!["prefix", "abc"]);
    }

    #[test]
    fn url_wins_over_path_inside_url() {
        let matches =
            find_matches_with_defaults_only(&lines(["open https://example.com/src/main.rs"]));

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].pattern, "url");
        assert_eq!(matches[0].text, "https://example.com/src/main.rs");
    }

    #[test]
    fn uuid_wins_over_sha_and_number_submatches() {
        let matches =
            find_matches_with_defaults_only(&lines(["id 123e4567-e89b-12d3-a456-426614174000"]));

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].pattern, "uuid");
    }

    #[test]
    fn git_sha_wins_over_number_submatch() {
        let matches = find_matches_with_defaults_only(&lines(["sha abc1234"]));

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].pattern, "git-sha");
        assert_eq!(matches[0].text, "abc1234");
    }

    #[test]
    fn higher_priority_beats_longer_lower_priority_overlap() {
        let patterns = vec![
            test_pattern("short-high", 10, r#"abc"#),
            test_pattern("long-low", 20, r#"abcdef"#),
        ];
        let matches = find_matches_with_patterns(&lines(["abcdef"]), &test_patterns(&patterns));

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].pattern, "short-high");
    }

    #[test]
    fn longer_match_wins_within_same_priority() {
        let patterns = vec![
            test_pattern("short", 10, r#"abc"#),
            test_pattern("long", 10, r#"abcdef"#),
        ];
        let matches = find_matches_with_patterns(&lines(["abcdef"]), &test_patterns(&patterns));

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].pattern, "long");
    }

    #[test]
    fn earlier_position_wins_within_same_priority_and_length() {
        let patterns = vec![test_pattern("word", 10, r#"abc"#)];
        let matches = find_matches_with_patterns(&lines(["abc abc"]), &test_patterns(&patterns));

        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].start, 0);
        assert_eq!(matches[1].start, 4);
    }

    #[test]
    fn pattern_definition_order_breaks_exact_ties() {
        let patterns = vec![
            test_pattern("first", 10, r#"abc"#),
            test_pattern("second", 10, r#"abc"#),
        ];
        let matches = find_matches_with_patterns(&lines(["abc"]), &test_patterns(&patterns));

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].pattern, "first");
    }

    #[test]
    fn output_order_is_visible_order_not_acceptance_priority() {
        let matches = find_matches_with_defaults_only(&lines(["1234", "https://example.com"]));
        let texts: Vec<&str> = matches.iter().map(|span| span.text.as_str()).collect();

        assert_eq!(texts, vec!["1234", "https://example.com"]);
    }

    #[test]
    fn duplicate_copied_text_occurrences_are_preserved() {
        let matches = find_matches_with_defaults_only(&lines(["/tmp/foo /tmp/foo"]));
        let paths: Vec<&MatchSpan> = matches
            .iter()
            .filter(|span| span.pattern == "path")
            .collect();

        assert_eq!(paths.len(), 2);
        assert_eq!(paths[0].text, paths[1].text);
        assert_ne!(paths[0].start, paths[1].start);
    }

    #[test]
    fn overlap_is_line_local() {
        let patterns = vec![test_pattern("same", 10, r#"abc"#)];
        let matches = find_matches_with_patterns(&lines(["abc", "abc"]), &test_patterns(&patterns));

        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].line, 0);
        assert_eq!(matches[1].line, 1);
    }

    #[test]
    fn disabled_builtin_patterns_are_skipped() {
        let enabled = vec!["url".to_string()];
        let matches = find_matches(
            &lines(["12345 https://example.com /tmp/path"]),
            &[],
            Some(&enabled),
        );
        let texts: Vec<&str> = matches.iter().map(|span| span.text.as_str()).collect();

        assert_eq!(texts, vec!["https://example.com"]);
    }

    #[test]
    fn disabled_builtins_do_not_affect_custom_patterns() {
        let custom = vec![CustomPatternDefinition::compile("ticket", 25, r#"ABC-[0-9]+"#).unwrap()];
        let enabled: Vec<String> = Vec::new();
        let matches = find_matches(&lines(["fix ABC-1234 in /tmp/x"]), &custom, Some(&enabled));
        let texts: Vec<&str> = matches.iter().map(|span| span.text.as_str()).collect();

        assert_eq!(texts, vec!["ABC-1234"]);
    }

    #[test]
    fn builtin_names_follow_the_priority_order() {
        assert_eq!(
            builtin_pattern_names(),
            [
                "url",
                "git-status",
                "git-status-branch",
                "diff",
                "kubernetes",
                "path",
                "uuid",
                "kubernetes-pod",
                "git-sha",
                "hex",
                "ipv4",
                "number",
            ]
        );
    }

    #[test]
    fn url_trimming_keeps_balanced_brackets_and_drops_unmatched_ones() {
        let matches = find_matches_with_defaults_only(&lines([
            "a https://example.com/x[1]] b https://example.com/y{2} c https://example.com/z}.",
        ]));
        let urls: Vec<&str> = matches
            .iter()
            .filter(|span| span.pattern == "url")
            .map(|span| span.text.as_str())
            .collect();

        assert_eq!(
            urls,
            vec![
                "https://example.com/x[1]",
                "https://example.com/y{2}",
                "https://example.com/z",
            ]
        );
    }

    #[test]
    fn custom_patterns_are_searched_before_builtins() {
        let custom = vec![CustomPatternDefinition::compile("ticket", 25, r#"ABC-[0-9]+"#).unwrap()];
        let matches = find_matches(&lines(["fix ABC-1234"]), &custom, None);

        assert!(matches
            .iter()
            .any(|span| span.pattern == "ticket" && span.text == "ABC-1234"));
    }

    #[test]
    fn custom_pattern_priority_participates_in_overlap_resolution() {
        let custom = vec![CustomPatternDefinition::compile(
            "command",
            12,
            r#"git branch -D [A-Za-z0-9._/-]+"#,
        )
        .unwrap()];
        let matches = find_matches(&lines(["run git branch -D feature/foo"]), &custom, None);

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].pattern, "command");
        assert_eq!(matches[0].text, "git branch -D feature/foo");
    }
}
