use crate::model::{ClipboardMode, PatternSpec, PickerSettings, Theme, ThemeColor};
use crate::patterns::{builtin_pattern_names, CustomPatternDefinition};
use crate::theme::parse_theme_color;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use std::fmt::Display;
use std::path::{Path, PathBuf};
use toml::{Table, Value};

/// Herdr plugin id used for config directory discovery outside plugin action env.
pub const PLUGIN_ID: &str = "pgilad.herdr-yank";

const CONFIG_FILE: &str = "config.toml";
const DEFAULT_PROJECT_CONFIG_FILE: &str = ".herdr-yank.toml";

/// Settings and patterns for one launch, plus each problem found in the config files.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LoadedConfig {
    /// Project patterns first, then global patterns.
    pub patterns: Vec<PatternSpec>,
    pub settings: PickerSettings,
    /// One message for each ignored key, entry, or file.
    pub problems: Vec<String>,
}

/// One custom regex pattern loaded from user or project configuration.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PatternConfigEntry {
    name: String,
    regex: String,
    #[serde(default = "default_custom_priority")]
    priority: u16,
}

fn default_custom_priority() -> u16 {
    25
}

/// Project-local pattern discovery settings from global config.
///
/// Project-local patterns are opt-in. A `.herdr-yank.toml` inside a checkout is
/// attacker-controlled content whenever the checkout is. A high-priority pattern there can
/// change which substring a hint copies, so the feature stays off until the user asks for it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ProjectConfig {
    patterns: bool,
    pattern_files: Vec<String>,
}

impl Default for ProjectConfig {
    fn default() -> Self {
        Self {
            patterns: false,
            pattern_files: vec![DEFAULT_PROJECT_CONFIG_FILE.to_string()],
        }
    }
}

/// Loads the global config and, when enabled, the nearest project pattern file.
///
/// A problem makes Herdr Yank ignore only the key, entry, or file that has it. All valid
/// settings still apply, and `LoadedConfig::problems` tells the user what was ignored.
pub fn load_config(cwd: Option<&Path>) -> LoadedConfig {
    let global_path = global_config_dir().map(|dir| dir.join(CONFIG_FILE));
    load_config_from(global_path.as_deref(), cwd)
}

fn load_config_from(global_path: Option<&Path>, cwd: Option<&Path>) -> LoadedConfig {
    let mut problems = Vec::new();
    let mut table = match global_path.map(|path| read_config_file(path, CONFIG_FILE, &mut problems))
    {
        Some(ConfigFile::Loaded(table)) => table,
        _ => Table::new(),
    };

    let mut file = FileProblems {
        label: CONFIG_FILE.to_string(),
        problems: &mut problems,
    };
    let project = take_project(&mut table, &mut file);
    let global_patterns = take_patterns(&mut table, &mut file);
    let enabled_builtin_patterns = take_enabled_builtins(&mut table, &mut file);
    let clipboard = file
        .take::<ClipboardMode>(&mut table, "clipboard", "clipboard")
        .unwrap_or_default();
    let theme = take_style(&mut table, &mut file);
    for key in table.keys() {
        file.push(format_args!("unknown key `{key}`"));
    }

    let mut patterns = Vec::new();
    if let (true, Some(cwd)) = (project.patterns, cwd) {
        patterns.extend(load_project_patterns(
            cwd,
            &project.pattern_files,
            &mut problems,
        ));
    }
    patterns.extend(global_patterns);

    LoadedConfig {
        patterns,
        settings: PickerSettings {
            enabled_builtin_patterns,
            theme,
            clipboard,
        },
        problems,
    }
}

/// Problems of one config file. Each message starts with the file label.
struct FileProblems<'a> {
    label: String,
    problems: &'a mut Vec<String>,
}

impl FileProblems<'_> {
    fn push(&mut self, detail: impl Display) {
        self.problems
            .push(one_line(&format!("{}: {detail}", self.label)));
    }

    /// Removes `key` from `table` and deserializes it, or reports why it cannot.
    fn take<T: DeserializeOwned>(&mut self, table: &mut Table, key: &str, name: &str) -> Option<T> {
        let value = table.remove(key)?;
        match value.try_into::<T>() {
            Ok(value) => Some(value),
            Err(error) => {
                self.push(format_args!("invalid `{name}`: {error}"));
                None
            }
        }
    }
}

/// Collapses a multi-line message, such as a TOML or regex error, into one line.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

enum ConfigFile {
    Missing,
    Invalid,
    Loaded(Table),
}

fn read_config_file(path: &Path, label: &str, problems: &mut Vec<String>) -> ConfigFile {
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return ConfigFile::Missing,
        Err(error) => {
            problems.push(format!("{label}: cannot read the file: {error}"));
            return ConfigFile::Invalid;
        }
    };
    match toml::from_str::<Table>(&content) {
        Ok(table) => ConfigFile::Loaded(table),
        Err(error) => {
            problems.push(one_line(&format!("{label}: {error}")));
            ConfigFile::Invalid
        }
    }
}

fn take_project(table: &mut Table, file: &mut FileProblems) -> ProjectConfig {
    let mut project = ProjectConfig::default();
    let Some(mut section) = file.take::<Table>(table, "project", "project") else {
        return project;
    };
    if let Some(enabled) = file.take(&mut section, "patterns", "project.patterns") {
        project.patterns = enabled;
    }
    if let Some(files) = file.take(&mut section, "pattern_files", "project.pattern_files") {
        project.pattern_files = files;
    }
    for key in section.keys() {
        file.push(format_args!("unknown key `project.{key}`"));
    }
    project
}

fn take_patterns(table: &mut Table, file: &mut FileProblems) -> Vec<PatternSpec> {
    let Some(entries) = file.take::<Vec<Value>>(table, "patterns", "patterns") else {
        return Vec::new();
    };
    entries
        .into_iter()
        .enumerate()
        .filter_map(|(index, entry)| pattern_spec(index, entry, file))
        .collect()
}

/// Validates one `[[patterns]]` entry, including its regex, before the picker needs it.
fn pattern_spec(index: usize, entry: Value, file: &mut FileProblems) -> Option<PatternSpec> {
    let entry: PatternConfigEntry = match entry.try_into() {
        Ok(entry) => entry,
        Err(error) => {
            file.push(format_args!("invalid `patterns[{index}]`: {error}"));
            return None;
        }
    };
    if let Err(error) =
        CustomPatternDefinition::compile(entry.name.clone(), entry.priority, &entry.regex)
    {
        file.push(format_args!(
            "`patterns[{index}]` ({}): invalid regex: {error}",
            entry.name
        ));
        return None;
    }
    Some(PatternSpec {
        name: entry.name,
        regex: entry.regex,
        priority: entry.priority,
    })
}

fn take_enabled_builtins(table: &mut Table, file: &mut FileProblems) -> Option<Vec<String>> {
    let names = file.take::<Vec<String>>(
        table,
        "enabled_builtin_patterns",
        "enabled_builtin_patterns",
    )?;
    let known = builtin_pattern_names();
    Some(
        names
            .into_iter()
            .filter(|name| {
                let is_known = known.contains(&name.as_str());
                if !is_known {
                    file.push(format_args!(
                        "unknown built-in pattern `{name}` in `enabled_builtin_patterns`; known names are {}",
                        known.join(", ")
                    ));
                }
                is_known
            })
            .collect(),
    )
}

/// Theme slot that one `[style]` key sets.
enum StyleSlot<'a> {
    Color(&'a mut ThemeColor),
    Optional(&'a mut Option<ThemeColor>),
}

fn style_slot<'a>(theme: &'a mut Theme, key: &str) -> Option<StyleSlot<'a>> {
    Some(match key {
        "unmatched_fg" => StyleSlot::Color(&mut theme.unmatched_fg),
        "match_fg" => StyleSlot::Color(&mut theme.match_fg),
        "match_bg" => StyleSlot::Optional(&mut theme.match_bg),
        "hint_fg" => StyleSlot::Color(&mut theme.hint_fg),
        "hint_bg" => StyleSlot::Color(&mut theme.hint_bg),
        "selected_match_fg" => StyleSlot::Color(&mut theme.selected_match_fg),
        "selected_match_bg" => StyleSlot::Optional(&mut theme.selected_match_bg),
        "selected_hint_fg" => StyleSlot::Color(&mut theme.selected_hint_fg),
        "selected_hint_bg" => StyleSlot::Color(&mut theme.selected_hint_bg),
        "status_fg" => StyleSlot::Color(&mut theme.status_fg),
        "status_bg" => StyleSlot::Color(&mut theme.status_bg),
        _ => return None,
    })
}

fn take_style(table: &mut Table, file: &mut FileProblems) -> Theme {
    let mut theme = Theme::default();
    let Some(style) = file.take::<Table>(table, "style", "style") else {
        return theme;
    };

    for (key, value) in style {
        let Some(slot) = style_slot(&mut theme, &key) else {
            file.push(format_args!("unknown key `style.{key}`"));
            continue;
        };
        let Some(text) = value.as_str() else {
            file.push(format_args!("`style.{key}` must be a color string"));
            continue;
        };
        let Some(color) = parse_theme_color(text) else {
            file.push(format_args!("`style.{key}`: {text:?} is not a known color"));
            continue;
        };
        match slot {
            StyleSlot::Color(slot) => *slot = color,
            StyleSlot::Optional(slot) => *slot = Some(color),
        }
    }
    theme
}

/// Compiles snapshot-provided custom pattern specs.
///
/// The launcher already rejected invalid regexes, so a failure here is only logged.
pub fn compile_pattern_specs(specs: &[PatternSpec]) -> Vec<CustomPatternDefinition> {
    specs
        .iter()
        .filter_map(|spec| {
            CustomPatternDefinition::compile(spec.name.clone(), spec.priority, &spec.regex)
                .inspect_err(|error| {
                    eprintln!(
                        "Herdr Yank: ignoring invalid pattern {}: {error}",
                        spec.name
                    );
                })
                .ok()
        })
        .collect()
}

/// Finds directory that holds `config.toml`.
///
/// Herdr sets `HERDR_PLUGIN_CONFIG_DIR` for a plugin action. A direct binary run and
/// `herdr plugin action invoke` do not, so the fallback rebuilds the same path that
/// `herdr plugin config-dir` prints. Without it the picker ignores the user config in
/// silence.
fn global_config_dir() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("HERDR_PLUGIN_CONFIG_DIR") {
        return Some(PathBuf::from(path));
    }
    Some(plugin_config_dir_in(&xdg_config_home()?))
}

/// Rebuilds the path that `herdr plugin config-dir <id>` prints.
fn plugin_config_dir_in(xdg_config_home: &Path) -> PathBuf {
    xdg_config_home
        .join("herdr")
        .join("plugins")
        .join("config")
        .join(PLUGIN_ID)
}

fn xdg_config_home() -> Option<PathBuf> {
    match std::env::var_os("XDG_CONFIG_HOME") {
        Some(value) if !value.is_empty() => Some(PathBuf::from(value)),
        _ => Some(PathBuf::from(std::env::var_os("HOME")?).join(".config")),
    }
}

/// Loads the nearest project pattern file between `cwd` and the Git root.
///
/// A project file holds only `[[patterns]]`. Other keys are problems, because a checkout
/// must not change the clipboard, style, or project settings of the user.
fn load_project_patterns(
    cwd: &Path,
    pattern_files: &[String],
    problems: &mut Vec<String>,
) -> Vec<PatternSpec> {
    let Some(git_root) = find_git_root(cwd) else {
        return Vec::new();
    };

    for dir in ancestors_until(cwd, &git_root) {
        for file_name in pattern_files {
            let path = dir.join(file_name);
            let label = path.display().to_string();
            match read_config_file(&path, &label, problems) {
                ConfigFile::Missing => continue,
                ConfigFile::Invalid => return Vec::new(),
                ConfigFile::Loaded(mut table) => {
                    let mut file = FileProblems { label, problems };
                    let patterns = take_patterns(&mut table, &mut file);
                    for key in table.keys() {
                        file.push(format_args!(
                            "unknown key `{key}`; a project file holds only `[[patterns]]`"
                        ));
                    }
                    return patterns;
                }
            }
        }
    }

    Vec::new()
}

fn find_git_root(cwd: &Path) -> Option<PathBuf> {
    cwd.ancestors()
        .find(|ancestor| ancestor.join(".git").exists())
        .map(Path::to_path_buf)
}

fn ancestors_until<'a>(cwd: &'a Path, root: &'a Path) -> impl Iterator<Item = &'a Path> {
    cwd.ancestors()
        .take_while(move |ancestor| ancestor.starts_with(root))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;

    /// Loads `content` as the global config file, with an optional project checkout.
    fn load(name: &str, content: &str, cwd: Option<&Path>) -> LoadedConfig {
        let dir = TempDir::new(name);
        let path = dir.join(CONFIG_FILE);
        std::fs::write(&path, content).unwrap();
        load_config_from(Some(&path), cwd)
    }

    fn project(name: &str, content: &str) -> TempDir {
        let root = TempDir::new(name);
        std::fs::create_dir(root.join(".git")).unwrap();
        std::fs::write(root.join(DEFAULT_PROJECT_CONFIG_FILE), content).unwrap();
        root
    }

    #[test]
    fn a_valid_config_applies_every_key_without_problems() {
        let config = load(
            "valid",
            r##"
clipboard = "osc52"
enabled_builtin_patterns = ["url", "git-sha"]

[[patterns]]
name = "ticket"
regex = "ABC-(?<match>[0-9]+)"

[style]
hint_bg = "#ffd75f"
match_bg = "ansi:236"
match_fg = "green"
"##,
            None,
        );

        assert!(config.problems.is_empty(), "{:?}", config.problems);
        assert_eq!(config.settings.clipboard, ClipboardMode::Osc52);
        assert_eq!(
            config.settings.enabled_builtin_patterns.as_deref(),
            Some(["url".to_string(), "git-sha".to_string()].as_slice())
        );
        assert_eq!(
            config.patterns,
            [PatternSpec {
                name: "ticket".into(),
                regex: "ABC-(?<match>[0-9]+)".into(),
                priority: 25,
            }]
        );
        let theme = config.settings.theme;
        assert_eq!(
            theme.hint_bg,
            ThemeColor::Rgb {
                r: 255,
                g: 215,
                b: 95
            }
        );
        assert_eq!(theme.match_bg, Some(ThemeColor::Ansi(236)));
        assert_eq!(theme.match_fg, ThemeColor::Green);
        assert_eq!(theme.hint_fg, Theme::default().hint_fg);
    }

    #[test]
    fn a_missing_config_file_is_not_a_problem() {
        let config = load_config_from(Some(Path::new("/nonexistent/herdr-yank.toml")), None);

        assert_eq!(config, LoadedConfig::default());
    }

    #[test]
    fn unknown_keys_are_reported_and_other_keys_still_apply() {
        let config = load(
            "unknown-keys",
            "clipbaord = \"tool\"\nclipboard = \"osc52\"\n[project]\npattern = true\n",
            None,
        );

        assert_eq!(config.settings.clipboard, ClipboardMode::Osc52);
        assert_eq!(
            config.problems,
            [
                "config.toml: unknown key `project.pattern`",
                "config.toml: unknown key `clipbaord`",
            ]
        );
    }

    #[test]
    fn an_invalid_color_resets_only_its_own_slot() {
        let config = load(
            "invalid-color",
            "clipboard = \"tool\"\n[style]\nhint_bg = \"pink\"\nmatch_fg = \"green\"\nborder = \"red\"\nstatus_fg = 3\n",
            None,
        );

        let theme = config.settings.theme;
        assert_eq!(config.settings.clipboard, ClipboardMode::Tool);
        assert_eq!(theme.hint_bg, Theme::default().hint_bg);
        assert_eq!(theme.match_fg, ThemeColor::Green);
        assert_eq!(
            config.problems,
            [
                "config.toml: unknown key `style.border`",
                "config.toml: `style.hint_bg`: \"pink\" is not a known color",
                "config.toml: `style.status_fg` must be a color string",
            ]
        );
    }

    #[test]
    fn unknown_builtin_names_are_dropped_and_known_names_stay() {
        let config = load(
            "unknown-builtins",
            "enabled_builtin_patterns = [\"url\", \"nope\"]\n",
            None,
        );

        assert_eq!(
            config.settings.enabled_builtin_patterns,
            Some(vec!["url".to_string()])
        );
        assert_eq!(config.problems.len(), 1);
        assert!(config.problems[0].contains("unknown built-in pattern `nope`"));
    }

    #[test]
    fn invalid_pattern_entries_are_skipped_one_by_one() {
        let config = load(
            "invalid-patterns",
            r#"
[[patterns]]
name = "good"
regex = "GOOD-[0-9]+"
priority = 12

[[patterns]]
name = "no-regex"

[[patterns]]
name = "bad-regex"
regex = "(unclosed"

[[patterns]]
name = "extra"
regex = "x"
colour = "red"
"#,
            None,
        );

        assert_eq!(
            config.patterns,
            [PatternSpec {
                name: "good".into(),
                regex: "GOOD-[0-9]+".into(),
                priority: 12,
            }]
        );
        assert_eq!(config.problems.len(), 3, "{:?}", config.problems);
        assert!(config.problems[0].starts_with("config.toml: invalid `patterns[1]`"));
        assert!(
            config.problems[1].starts_with("config.toml: `patterns[2]` (bad-regex): invalid regex")
        );
        assert!(config.problems[2].starts_with("config.toml: invalid `patterns[3]`"));
        assert!(config
            .problems
            .iter()
            .all(|problem| !problem.contains('\n')));
    }

    #[test]
    fn wrong_value_types_are_reported_and_keep_the_defaults() {
        let config = load(
            "wrong-types",
            "clipboard = 5\nenabled_builtin_patterns = \"url\"\n[project]\npatterns = \"yes\"\n",
            None,
        );

        assert_eq!(config.settings, PickerSettings::default());
        assert_eq!(config.problems.len(), 3, "{:?}", config.problems);
        assert!(config.problems[0].contains("invalid `project.patterns`"));
        assert!(config.problems[1].contains("invalid `enabled_builtin_patterns`"));
        assert!(config.problems[2].contains("invalid `clipboard`"));
    }

    #[test]
    fn a_syntax_error_keeps_all_defaults_and_is_reported() {
        let config = load("syntax-error", "clipboard = \n", None);

        assert_eq!(config.settings, PickerSettings::default());
        assert_eq!(config.problems.len(), 1);
        assert!(config.problems[0].starts_with("config.toml: TOML parse error"));
    }

    #[test]
    fn project_patterns_load_only_when_explicitly_enabled() {
        let root = project(
            "project-opt-in",
            "[[patterns]]\nname = \"p\"\nregex = \"P-[0-9]+\"\n",
        );

        let disabled = load("project-opt-in-off", "", Some(&root));
        assert!(disabled.patterns.is_empty());

        let enabled = load(
            "project-opt-in-on",
            "[project]\npatterns = true\n",
            Some(&root),
        );
        assert_eq!(enabled.patterns.len(), 1);
        assert!(enabled.problems.is_empty());
    }

    #[test]
    fn project_patterns_come_before_global_patterns() {
        let root = project(
            "project-order",
            "[[patterns]]\nname = \"project\"\nregex = \"P-[0-9]+\"\n",
        );

        let config = load(
            "project-order-global",
            "[project]\npatterns = true\n[[patterns]]\nname = \"global\"\nregex = \"G-[0-9]+\"\n",
            Some(&root),
        );

        let names: Vec<&str> = config.patterns.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["project", "global"]);
    }

    #[test]
    fn project_config_is_discovered_up_to_git_root() {
        let root = project(
            "project-discovery",
            "[[patterns]]\nname = \"project\"\nregex = \"PROJECT-[0-9]+\"\n",
        );
        let nested = root.join("a/b");
        std::fs::create_dir_all(&nested).unwrap();
        let mut problems = Vec::new();

        let specs = load_project_patterns(
            &nested,
            &[DEFAULT_PROJECT_CONFIG_FILE.to_string()],
            &mut problems,
        );

        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].name, "project");
        assert!(problems.is_empty());
    }

    #[test]
    fn a_project_file_holds_only_patterns() {
        let root = project(
            "project-extra-keys",
            "clipboard = \"tool\"\n[[patterns]]\nname = \"p\"\nregex = \"P-[0-9]+\"\n[style]\nhint_bg = \"red\"\n",
        );

        let config = load(
            "project-extra-keys-global",
            "[project]\npatterns = true\n",
            Some(&root),
        );

        assert_eq!(config.patterns.len(), 1);
        assert_eq!(config.settings.clipboard, ClipboardMode::Auto);
        assert_eq!(config.settings.theme, Theme::default());
        assert_eq!(config.problems.len(), 2, "{:?}", config.problems);
        assert!(config
            .problems
            .iter()
            .all(|problem| problem.ends_with("a project file holds only `[[patterns]]`")));
    }

    #[test]
    fn an_invalid_project_file_is_ignored_and_reported() {
        let root = project("invalid-project", "not toml =");
        let mut problems = Vec::new();

        let specs = load_project_patterns(
            &root,
            &[DEFAULT_PROJECT_CONFIG_FILE.to_string()],
            &mut problems,
        );

        assert!(specs.is_empty());
        assert_eq!(problems.len(), 1);
        assert!(problems[0].contains(DEFAULT_PROJECT_CONFIG_FILE));
    }

    #[test]
    fn config_dir_falls_back_to_the_herdr_plugin_config_path() {
        assert_eq!(
            plugin_config_dir_in(Path::new("/home/u/.config")),
            PathBuf::from("/home/u/.config/herdr/plugins/config/pgilad.herdr-yank")
        );
    }
}
