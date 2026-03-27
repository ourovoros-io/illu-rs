use lsp_types::{Location, Position, SymbolKind, Url};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::str::FromStr;

use super::error::{RaError, Result};

/// A position in a file, specified as `file:line:col` (1-indexed for human input).
#[derive(Debug, Clone)]
pub struct PositionSpec {
    pub file: PathBuf,
    pub line: u32,
    pub col: u32,
}

impl PositionSpec {
    #[must_use]
    pub fn new(file: PathBuf, line: u32, col: u32) -> Self {
        Self { file, line, col }
    }

    /// Convert to LSP Position (0-indexed).
    #[must_use]
    pub fn to_lsp_position(&self) -> Position {
        Position {
            line: self.line.saturating_sub(1),
            character: self.col.saturating_sub(1),
        }
    }

    /// Convert file path to a file:// URL.
    pub fn to_url(&self) -> Result<Url> {
        let abs = if self.file.is_absolute() {
            self.file.clone()
        } else {
            std::env::current_dir()?.join(&self.file)
        };
        Url::from_file_path(&abs).map_err(|()| {
            RaError::InvalidPosition(format!("cannot convert to URL: {}", abs.display()))
        })
    }
}

impl FromStr for PositionSpec {
    type Err = RaError;

    fn from_str(s: &str) -> Result<Self> {
        let parts: Vec<&str> = s.rsplitn(3, ':').collect();
        match parts.as_slice() {
            [col, line, file] => {
                let line: u32 = line
                    .parse()
                    .map_err(|_| RaError::InvalidPosition(format!("invalid line: {line}")))?;
                let col: u32 = col
                    .parse()
                    .map_err(|_| RaError::InvalidPosition(format!("invalid column: {col}")))?;
                Ok(PositionSpec::new(PathBuf::from(file), line, col))
            }
            _ => Err(RaError::InvalidPosition(format!(
                "expected file:line:col, got: {s}"
            ))),
        }
    }
}

/// A location with enriched context for display.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RichLocation {
    pub file: String,
    pub line: u32,
    pub col: u32,
    pub end_line: u32,
    pub end_col: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_before: Option<Vec<String>>,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_after: Option<Vec<String>>,
}

impl RichLocation {
    #[must_use]
    pub fn from_location(loc: &Location) -> Self {
        let file = url_to_path_string(&loc.uri);
        RichLocation {
            file,
            line: loc.range.start.line + 1,
            col: loc.range.start.character + 1,
            end_line: loc.range.end.line + 1,
            end_col: loc.range.end.character + 1,
            context_before: None,
            text: String::new(),
            context_after: None,
        }
    }
}

/// Symbol information for outline/workspace symbol results.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspSymbolInfo {
    pub name: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub file: String,
    pub line: u32,
    pub col: u32,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<LspSymbolInfo>,
}

/// Call hierarchy item for display.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallInfo {
    pub name: String,
    pub kind: String,
    pub file: String,
    pub line: u32,
    pub col: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub call_sites: Vec<CallSite>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallSite {
    pub file: String,
    pub line: u32,
    pub col: u32,
}

/// Composed symbol context result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SymbolContext {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hover: Option<String>,
    pub definition: RichLocation,
    pub reference_count: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub incoming_calls: Vec<CallInfo>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub outgoing_calls: Vec<CallInfo>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub implementations: Vec<RichLocation>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub related_tests: Vec<String>,
}

/// Rename impact preview.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenameImpact {
    pub old_name: String,
    pub new_name: String,
    pub files_affected: Vec<String>,
    pub total_references: usize,
    pub references_by_file: Vec<FileReferences>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileReferences {
    pub file: String,
    pub count: usize,
    pub lines: Vec<u32>,
}

/// Rename result after applying.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenameResult {
    pub old_name: String,
    pub new_name: String,
    pub files_changed: Vec<String>,
    pub total_edits: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub new_diagnostics: Vec<DiagnosticInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticInfo {
    pub file: String,
    pub line: u32,
    pub severity: String,
    pub message: String,
}

/// Convert a file URL to a display path string.
#[must_use]
pub fn url_to_path_string(url: &Url) -> String {
    url.to_file_path()
        .map_or_else(|()| url.to_string(), |p| p.display().to_string())
}

/// Convert a `SymbolKind` to a human-readable string.
#[must_use]
pub fn symbol_kind_name(kind: SymbolKind) -> &'static str {
    match kind {
        SymbolKind::FILE => "file",
        SymbolKind::MODULE => "module",
        SymbolKind::NAMESPACE => "namespace",
        SymbolKind::PACKAGE => "package",
        SymbolKind::CLASS => "class",
        SymbolKind::METHOD => "method",
        SymbolKind::PROPERTY => "property",
        SymbolKind::FIELD => "field",
        SymbolKind::CONSTRUCTOR => "constructor",
        SymbolKind::ENUM => "enum",
        SymbolKind::INTERFACE => "interface",
        SymbolKind::FUNCTION => "function",
        SymbolKind::VARIABLE => "variable",
        SymbolKind::CONSTANT => "constant",
        SymbolKind::STRING => "string",
        SymbolKind::NUMBER => "number",
        SymbolKind::BOOLEAN => "boolean",
        SymbolKind::ARRAY => "array",
        SymbolKind::OBJECT => "object",
        SymbolKind::KEY => "key",
        SymbolKind::NULL => "null",
        SymbolKind::ENUM_MEMBER => "enum_member",
        SymbolKind::STRUCT => "struct",
        SymbolKind::EVENT => "event",
        SymbolKind::OPERATOR => "operator",
        SymbolKind::TYPE_PARAMETER => "type_parameter",
        _ => "unknown",
    }
}

/// Read surrounding lines of context from a file around a given line.
pub fn read_context_lines(
    path: &Path,
    target_line: u32,
    context: u32,
) -> std::io::Result<(Vec<String>, String, Vec<String>)> {
    let content = std::fs::read_to_string(path)?;
    let lines: Vec<&str> = content.lines().collect();
    if lines.is_empty() {
        return Ok((vec![], String::new(), vec![]));
    }
    let idx = (target_line.saturating_sub(1) as usize).min(lines.len() - 1);

    let start = idx.saturating_sub(context as usize);
    let end = (idx + context as usize + 1).min(lines.len());

    let before = lines[start..idx].iter().map(|s| (*s).to_string()).collect();
    let text = (*lines.get(idx).unwrap_or(&"")).to_string();
    let after_start = (idx + 1).min(lines.len());
    let after = lines[after_start..end]
        .iter()
        .map(|s| (*s).to_string())
        .collect();

    Ok((before, text, after))
}
