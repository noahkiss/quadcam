//! The schema version of the MCP and CLI surface, and the rules that keep it stable.
//!
//! From 1.0 the surface (MCP tools, their `action` values and parameters; CLI commands, flags
//! and positional arguments) only grows. A removal or a rename needs a [`SCHEMA_VERSION`] bump
//! and a [`Deprecation`] listed here for at least one version before it. `tests/surface.rs`
//! compares the surface with the baseline in `tests/surface/` through [`check`].

use serde_json::{json, Value};
use std::collections::BTreeSet;

/// The version of the MCP and CLI surface. It starts at 1 and counts removals and renames only:
/// an added tool, action, parameter, command or flag leaves it alone. The MCP server reports it
/// in `serverInfo.schemaVersion` and in `quadcam_status`; `quadcam-cli --schema-version` prints it.
pub const SCHEMA_VERSION: u32 = 1;

/// One item of the surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Item {
    /// An MCP tool.
    McpTool(&'static str),
    /// One value of an MCP tool's `action` parameter.
    McpAction {
        tool: &'static str,
        action: &'static str,
    },
    /// An MCP tool's parameter.
    McpParam {
        tool: &'static str,
        param: &'static str,
    },
    /// A CLI command by its path, for example `gear fc read`.
    CliCommand(&'static str),
    /// A CLI flag of a command (`""` for the root), for example `("gear fc read", "--port")`.
    CliFlag {
        command: &'static str,
        flag: &'static str,
    },
}

impl Item {
    /// The item's line in the baseline files.
    pub fn key(&self) -> String {
        match self {
            Item::McpTool(t) => format!("mcp.tool {t}"),
            Item::McpAction { tool, action } => format!("mcp.action {tool}.{action}"),
            Item::McpParam { tool, param } => format!("mcp.param {tool}.{param}"),
            Item::CliCommand(c) => format!("cli.command {c}"),
            Item::CliFlag { command, flag } => format!("cli.flag {command} {flag}"),
        }
    }

    /// Whether this item stands behind a line of the surface: the item's own line, and the lines
    /// below it (a tool's actions and parameters, a command's flags and subcommands).
    pub fn covers(&self, line: &str) -> bool {
        match self {
            Item::McpTool(t) => {
                line == format!("mcp.tool {t}")
                    || ["action", "param", "value", "required"]
                        .iter()
                        .any(|k| line.starts_with(&format!("mcp.{k} {t}.")))
            }
            Item::McpAction { tool, action } => {
                line == format!("mcp.action {tool}.{action}")
                    || line.starts_with(&format!("mcp.value {tool}.{action}="))
            }
            Item::McpParam { tool, param } => {
                line == format!("mcp.required {tool}.{param}")
                    || line.starts_with(&format!("mcp.param {tool}.{param} "))
                    || line.starts_with(&format!("mcp.value {tool}.{param}="))
            }
            Item::CliCommand(c) => {
                line == format!("cli.command {c}")
                    || ["command", "flag", "arg", "value"]
                        .iter()
                        .any(|k| line.starts_with(&format!("cli.{k} {c} ")))
            }
            Item::CliFlag { command, flag } => {
                line == format!("cli.flag {command} {flag}")
                    || line.starts_with(&format!("cli.flag {command} {flag} "))
                    || line.starts_with(&format!("cli.value {command} {flag}="))
            }
        }
    }

    /// The item as a person reads it in a warning.
    pub fn describe(&self) -> String {
        match self {
            Item::McpTool(t) => format!("the MCP tool {t}"),
            Item::McpAction { tool, action } => format!("{tool} action={action}"),
            Item::McpParam { tool, param } => format!("the {param} parameter of {tool}"),
            Item::CliCommand(c) => format!("the command `quadcam-cli {c}`"),
            Item::CliFlag { command, flag } => {
                format!("the flag {flag} of `quadcam-cli {command}`")
            }
        }
    }
}

/// A surface item that still works, and the one that replaces it.
#[derive(Debug, Clone, Copy)]
pub struct Deprecation {
    pub item: Item,
    /// What to use instead, in words an agent can act on.
    pub replacement: &'static str,
    /// The schema version that deprecated it. The item may go in a later version, never in this one.
    pub since: u32,
}

impl Deprecation {
    /// The warning an MCP result or a CLI run carries.
    pub fn warning(&self) -> String {
        format!(
            "Deprecated since schema {}: {} keeps working but a later schema version removes it. Use {}.",
            self.since,
            self.item.describe(),
            self.replacement
        )
    }
}

/// Everything deprecated now. Adding one is the first step of a removal or a rename; the item
/// keeps working until a later [`SCHEMA_VERSION`] drops it.
pub const DEPRECATIONS: &[Deprecation] = &[];

// ----- runtime: warnings -----

/// The deprecations an MCP call touches: its tool, its `action`, or a parameter it passes.
pub fn mcp_deprecations<'a>(
    table: &'a [Deprecation],
    tool: &str,
    args: &Value,
) -> Vec<&'a Deprecation> {
    table
        .iter()
        .filter(|d| match d.item {
            Item::McpTool(t) => t == tool,
            Item::McpAction { tool: t, action } => {
                t == tool && args.get("action").and_then(Value::as_str) == Some(action)
            }
            Item::McpParam { tool: t, param } => t == tool && args.get(param).is_some(),
            _ => false,
        })
        .collect()
}

/// The deprecations a command line touches. `argv` is the arguments after the program name.
/// A command matches when its words open the arguments that are not flags; a flag matches
/// when it appears as `--flag` or `--flag=value`.
pub fn cli_deprecations<'a>(table: &'a [Deprecation], argv: &[String]) -> Vec<&'a Deprecation> {
    let mut words = Vec::new();
    let mut skip = false;
    for a in argv {
        if skip {
            skip = false;
        } else if a == "--session" {
            skip = true;
        } else if !a.starts_with('-') {
            words.push(a.as_str());
        }
    }
    let line = words.join(" ");
    let under = |c: &str| line == c || line.starts_with(&format!("{c} "));
    table
        .iter()
        .filter(|d| match d.item {
            Item::CliCommand(c) => under(c),
            Item::CliFlag { command, flag } => {
                (command.is_empty() || under(command))
                    && argv
                        .iter()
                        .any(|a| a == flag || a.starts_with(&format!("{flag}=")))
            }
            _ => false,
        })
        .collect()
}

/// Marks deprecated items in a tool list: a note on the tool's description, and
/// `"deprecated": true` on a parameter.
pub fn annotate_tools(tools: &mut Value, table: &[Deprecation]) {
    let Some(list) = tools.as_array_mut() else {
        return;
    };
    for tool in list {
        let name = tool["name"].as_str().unwrap_or("").to_string();
        let mut notes = Vec::new();
        for d in table {
            match d.item {
                Item::McpTool(t) if t == name => notes.push(d.warning()),
                Item::McpAction { tool: t, .. } if t == name => notes.push(d.warning()),
                Item::McpParam { tool: t, param } if t == name => {
                    notes.push(d.warning());
                    if let Some(p) = tool["inputSchema"]["properties"].get_mut(param) {
                        p["deprecated"] = json!(true);
                    }
                }
                _ => {}
            }
        }
        if !notes.is_empty() {
            let text = tool["description"].as_str().unwrap_or("");
            tool["description"] = json!(format!("{text}\n\n{}", notes.join("\n")));
        }
    }
}

// ----- the surface as lines -----

/// The MCP surface as sorted lines: one per tool, `action` value and parameter (with its type
/// and enum values), and one per required parameter.
pub fn mcp_surface(tools: &Value) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for tool in tools.as_array().into_iter().flatten() {
        let name = tool["name"].as_str().unwrap_or("?");
        out.insert(format!("mcp.tool {name}"));
        let schema = &tool["inputSchema"];
        for r in schema["required"].as_array().into_iter().flatten() {
            out.insert(format!("mcp.required {name}.{}", r.as_str().unwrap_or("?")));
        }
        let Some(props) = schema["properties"].as_object() else {
            continue;
        };
        for (p, v) in props {
            out.insert(format!("mcp.param {name}.{p} {}", type_of(v)));
            let values = v
                .get("enum")
                .or_else(|| v["items"].get("enum"))
                .and_then(Value::as_array);
            for e in values.into_iter().flatten() {
                let e = e.as_str().map_or_else(|| e.to_string(), str::to_string);
                if p == "action" {
                    out.insert(format!("mcp.action {name}.{e}"));
                } else {
                    out.insert(format!("mcp.value {name}.{p}={e}"));
                }
            }
        }
    }
    out
}

/// A property's type as one word: `string`, `array<string>`, `integer|null`, `any`.
fn type_of(v: &Value) -> String {
    let one = |t: &Value| t.as_str().unwrap_or("any").to_string();
    let base = match &v["type"] {
        Value::Array(a) => a.iter().map(one).collect::<Vec<_>>().join("|"),
        Value::Null => "any".into(),
        t => one(t),
    };
    if base == "array" {
        if let Some(items) = v.get("items").filter(|i| i.is_object()) {
            return format!("array<{}>", type_of(items));
        }
    }
    base
}

/// Which surface a baseline or a listing covers.
pub fn render(version: u32, lines: &BTreeSet<String>) -> String {
    let mut s = format!("# schema_version {version}\n");
    for l in lines {
        s.push_str(l);
        s.push('\n');
    }
    s
}

/// A baseline file: its schema version and its lines.
pub fn parse_baseline(text: &str) -> (u32, BTreeSet<String>) {
    let mut version = 0;
    let mut lines = BTreeSet::new();
    for l in text.lines().filter(|l| !l.is_empty()) {
        match l.strip_prefix("# schema_version ") {
            Some(v) => version = v.trim().parse().unwrap_or(0),
            None => {
                lines.insert(l.to_string());
            }
        }
    }
    (version, lines)
}

// ----- the guard -----

/// What the surface did since the baseline.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// Gone (or changed) with no schema bump and deprecation behind it: fails the guard.
    pub removed: Vec<String>,
    /// Gone, and allowed: the version moved on and the item was deprecated.
    pub removed_allowed: Vec<String>,
    /// A parameter that became required: breaks callers, so it fails the guard.
    pub newly_required: Vec<String>,
    /// New items: allowed, but the baseline wants updating.
    pub added: Vec<String>,
}

impl Report {
    pub fn ok(&self) -> bool {
        self.removed.is_empty() && self.newly_required.is_empty()
    }
}

/// Compares the surface now with the baseline. An item the baseline has and the surface lacks
/// (a removal, a rename, a type change) passes only when `version` is above the baseline's
/// version and `table` deprecates the item at a version below `version`.
pub fn check(
    baseline: (u32, &BTreeSet<String>),
    current: &BTreeSet<String>,
    version: u32,
    table: &[Deprecation],
) -> Report {
    let (base_version, base) = baseline;
    let mut r = Report::default();
    for gone in base.difference(current) {
        // A parameter that stopped being required loosens the contract.
        if gone.starts_with("mcp.required ") {
            continue;
        }
        let allowed = version > base_version
            && table
                .iter()
                .any(|d| d.since < version && d.item.covers(gone));
        if allowed {
            r.removed_allowed.push(gone.clone());
        } else {
            r.removed.push(gone.clone());
        }
    }
    for new in current.difference(base) {
        if new.starts_with("mcp.required ") {
            r.newly_required.push(new.clone());
        } else {
            r.added.push(new.clone());
        }
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(lines: &[&str]) -> BTreeSet<String> {
        lines.iter().map(|s| s.to_string()).collect()
    }

    const OLD: Deprecation = Deprecation {
        item: Item::McpAction {
            tool: "quadcam_x",
            action: "old",
        },
        replacement: "quadcam_x action=new",
        since: 1,
    };

    #[test]
    fn an_added_item_passes_and_is_reported() {
        let base = set(&["mcp.tool quadcam_x"]);
        let now = set(&["mcp.tool quadcam_x", "mcp.tool quadcam_y"]);
        let r = check((1, &base), &now, 1, &[]);
        assert!(r.ok());
        assert_eq!(r.added, ["mcp.tool quadcam_y"]);
    }

    #[test]
    fn a_removal_fails_without_a_bump_and_a_deprecation() {
        let base = set(&["mcp.action quadcam_x.old"]);
        let now = set(&[]);
        for (version, table) in [(1, &[OLD][..]), (2, &[][..])] {
            let r = check((1, &base), &now, version, table);
            assert!(!r.ok(), "version {version}");
            assert_eq!(r.removed, ["mcp.action quadcam_x.old"]);
        }
    }

    #[test]
    fn a_deprecated_removal_passes_after_a_bump() {
        let base = set(&["mcp.action quadcam_x.old"]);
        let r = check((1, &base), &set(&[]), 2, &[OLD]);
        assert!(r.ok());
        assert_eq!(r.removed_allowed, ["mcp.action quadcam_x.old"]);
    }

    #[test]
    fn a_rename_is_a_removal_and_an_addition() {
        let base = set(&["mcp.param quadcam_x.log_dir string"]);
        let now = set(&["mcp.param quadcam_x.logs string"]);
        let r = check((1, &base), &now, 1, &[]);
        assert!(!r.ok());
        assert_eq!(r.added.len(), 1);
    }

    #[test]
    fn a_newly_required_parameter_fails() {
        let base = set(&["mcp.tool quadcam_x"]);
        let now = set(&["mcp.tool quadcam_x", "mcp.required quadcam_x.ids"]);
        let r = check((1, &base), &now, 1, &[]);
        assert!(!r.ok());
        assert_eq!(r.newly_required, ["mcp.required quadcam_x.ids"]);
    }

    #[test]
    fn a_loosened_requirement_passes() {
        let base = set(&["mcp.required quadcam_x.ids"]);
        let r = check((1, &base), &set(&[]), 1, &[]);
        assert!(r.ok());
    }

    #[test]
    fn the_baseline_round_trips() {
        let lines = set(&["cli.command gear", "mcp.tool quadcam_x"]);
        let (v, back) = parse_baseline(&render(3, &lines));
        assert_eq!((v, back), (3, lines));
    }

    #[test]
    fn mcp_warnings_match_tool_action_and_param() {
        let table = [
            OLD,
            Deprecation {
                item: Item::McpParam {
                    tool: "quadcam_x",
                    param: "gone",
                },
                replacement: "the stay parameter",
                since: 1,
            },
        ];
        let hit = |args: Value| mcp_deprecations(&table, "quadcam_x", &args).len();
        assert_eq!(hit(json!({"action": "old"})), 1);
        assert_eq!(hit(json!({"action": "new"})), 0);
        assert_eq!(hit(json!({"action": "new", "gone": 1})), 1);
        assert!(mcp_deprecations(&table, "quadcam_y", &json!({"action": "old"})).is_empty());
        assert!(table[0].warning().contains("quadcam_x action=old"));
    }

    #[test]
    fn cli_warnings_match_command_and_flag() {
        let table = [
            Deprecation {
                item: Item::CliCommand("gear map"),
                replacement: "gear switch-map",
                since: 1,
            },
            Deprecation {
                item: Item::CliFlag {
                    command: "import",
                    flag: "--output",
                },
                replacement: "--output-dir",
                since: 1,
            },
        ];
        let v = |a: &[&str]| -> Vec<String> { a.iter().map(|s| s.to_string()).collect() };
        assert_eq!(
            cli_deprecations(&table, &v(&["gear", "map", "--live"])).len(),
            1
        );
        assert_eq!(
            cli_deprecations(
                &table,
                &v(&["--json", "--session", "map", "gear", "status"])
            )
            .len(),
            0
        );
        assert_eq!(cli_deprecations(&table, &v(&["gear", "mapx"])).len(), 0);
        assert_eq!(
            cli_deprecations(&table, &v(&["import", "--output=/x"])).len(),
            1
        );
        assert_eq!(
            cli_deprecations(&table, &v(&["verify", "--output"])).len(),
            0
        );
    }

    #[test]
    fn annotate_marks_the_description_and_the_parameter() {
        let mut tools = json!([{
            "name": "quadcam_x",
            "description": "Does x.",
            "inputSchema": {"properties": {"gone": {"type": "string"}}},
        }]);
        let table = [Deprecation {
            item: Item::McpParam {
                tool: "quadcam_x",
                param: "gone",
            },
            replacement: "stay",
            since: 1,
        }];
        annotate_tools(&mut tools, &table);
        assert_eq!(
            tools[0]["inputSchema"]["properties"]["gone"]["deprecated"],
            true
        );
        assert!(tools[0]["description"]
            .as_str()
            .unwrap()
            .contains("Use stay"));
    }

    #[test]
    fn the_surface_lists_tools_actions_params_and_required() {
        let tools = json!([{
            "name": "quadcam_x",
            "inputSchema": {
                "required": ["action"],
                "properties": {
                    "action": {"type": "string", "enum": ["a", "b"]},
                    "ids": {"type": "array", "items": {"type": "string"}},
                    "mode": {"type": "string", "enum": ["fast"]},
                },
            },
        }]);
        let s = mcp_surface(&tools);
        for line in [
            "mcp.tool quadcam_x",
            "mcp.required quadcam_x.action",
            "mcp.param quadcam_x.action string",
            "mcp.action quadcam_x.a",
            "mcp.param quadcam_x.ids array<string>",
            "mcp.value quadcam_x.mode=fast",
        ] {
            assert!(s.contains(line), "{line}\n{s:#?}");
        }
    }
}
