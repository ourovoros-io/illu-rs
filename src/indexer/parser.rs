use tree_sitter::{Node, Parser};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SymbolKind {
    Function,
    Struct,
    Enum,
    Trait,
    Impl,
    Use,
    Mod,
}

impl std::fmt::Display for SymbolKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Function => "function",
            Self::Struct => "struct",
            Self::Enum => "enum",
            Self::Trait => "trait",
            Self::Impl => "impl",
            Self::Use => "use",
            Self::Mod => "mod",
        };
        f.write_str(s)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Visibility {
    Public,
    PublicCrate,
    Private,
}

impl std::fmt::Display for Visibility {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Public => "public",
            Self::PublicCrate => "pub(crate)",
            Self::Private => "private",
        };
        f.write_str(s)
    }
}

#[derive(Debug, Clone)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    pub visibility: Visibility,
    pub file_path: String,
    pub line_start: usize,
    pub line_end: usize,
    pub signature: String,
}

pub fn parse_rust_source(source: &str, file_path: &str) -> Result<Vec<Symbol>, String> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .map_err(|e| format!("Failed to set language: {e}"))?;

    let tree = parser.parse(source, None).ok_or("Failed to parse source")?;

    let root = tree.root_node();
    let mut symbols = Vec::new();
    extract_symbols(&root, source, file_path, &mut symbols);
    Ok(symbols)
}

fn extract_symbols(node: &Node, source: &str, file_path: &str, symbols: &mut Vec<Symbol>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_item" => {
                if let Some(sym) = extract_function(&child, source, file_path) {
                    symbols.push(sym);
                }
            }
            "struct_item" => {
                if let Some(sym) = extract_named_item(&child, source, file_path, SymbolKind::Struct)
                {
                    symbols.push(sym);
                }
            }
            "enum_item" => {
                if let Some(sym) = extract_named_item(&child, source, file_path, SymbolKind::Enum) {
                    symbols.push(sym);
                }
            }
            "trait_item" => {
                if let Some(sym) = extract_named_item(&child, source, file_path, SymbolKind::Trait)
                {
                    symbols.push(sym);
                }
            }
            "impl_item" => {
                let type_name = extract_impl_type(&child, source);
                let vis = get_visibility(&child, source);
                let sig = get_first_line(&child, source);
                symbols.push(Symbol {
                    name: type_name.clone().unwrap_or_default(),
                    kind: SymbolKind::Impl,
                    visibility: vis,
                    file_path: file_path.to_string(),
                    line_start: child.start_position().row + 1,
                    line_end: child.end_position().row + 1,
                    signature: sig,
                });
                extract_symbols(&child, source, file_path, symbols);
            }
            "use_declaration" => {
                let text = node_text(&child, source);
                symbols.push(Symbol {
                    name: text.clone(),
                    kind: SymbolKind::Use,
                    visibility: get_visibility(&child, source),
                    file_path: file_path.to_string(),
                    line_start: child.start_position().row + 1,
                    line_end: child.end_position().row + 1,
                    signature: text,
                });
            }
            "mod_item" => {
                if let Some(sym) = extract_named_item(&child, source, file_path, SymbolKind::Mod) {
                    symbols.push(sym);
                }
            }
            "declaration_list" => {
                extract_symbols(&child, source, file_path, symbols);
            }
            _ => {}
        }
    }
}

fn extract_function(node: &Node, source: &str, file_path: &str) -> Option<Symbol> {
    let name = find_child_by_kind(node, "identifier")?;
    let name_text = node_text(&name, source);
    let vis = get_visibility(node, source);
    let sig = get_first_line(node, source);

    Some(Symbol {
        name: name_text,
        kind: SymbolKind::Function,
        visibility: vis,
        file_path: file_path.to_string(),
        line_start: node.start_position().row + 1,
        line_end: node.end_position().row + 1,
        signature: sig,
    })
}

fn extract_named_item(
    node: &Node,
    source: &str,
    file_path: &str,
    kind: SymbolKind,
) -> Option<Symbol> {
    let name_node = find_child_by_kind(node, "type_identifier")
        .or_else(|| find_child_by_kind(node, "identifier"))?;
    let name = node_text(&name_node, source);
    let vis = get_visibility(node, source);
    let sig = get_first_line(node, source);

    Some(Symbol {
        name,
        kind,
        visibility: vis,
        file_path: file_path.to_string(),
        line_start: node.start_position().row + 1,
        line_end: node.end_position().row + 1,
        signature: sig,
    })
}

fn extract_impl_type(node: &Node, source: &str) -> Option<String> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .find(|child| child.kind() == "type_identifier" || child.kind() == "generic_type")
        .map(|child| node_text(&child, source))
}

fn get_visibility(node: &Node, source: &str) -> Visibility {
    let Some(vis_node) = find_child_by_kind(node, "visibility_modifier") else {
        return Visibility::Private;
    };
    let text = node_text(&vis_node, source);
    if text.contains("crate") {
        Visibility::PublicCrate
    } else {
        Visibility::Public
    }
}

fn find_child_by_kind<'a>(node: &'a Node, kind: &str) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .find(|child| child.kind() == kind)
}

fn node_text(node: &Node, source: &str) -> String {
    source[node.byte_range()].to_string()
}

fn get_first_line(node: &Node, source: &str) -> String {
    let text = node_text(node, source);
    text.lines().next().unwrap_or(&text).trim().to_string()
}

/// A reference from one symbol to another, identified by name.
#[derive(Debug, Clone)]
pub struct SymbolRef {
    /// Name of the symbol that contains the reference
    pub source_name: String,
    /// File path of the source symbol
    pub source_file: String,
    /// Name of the referenced symbol
    pub target_name: String,
    /// Kind of reference
    pub kind: RefKind,
}

#[derive(Debug, Clone)]
pub enum RefKind {
    /// Type used in signature or body
    TypeRef,
    /// Function/method call
    Call,
}

impl std::fmt::Display for RefKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TypeRef => f.write_str("type_ref"),
            Self::Call => f.write_str("call"),
        }
    }
}

/// Extract references between symbols by scanning function/impl bodies
/// for identifiers that match known symbol names.
pub fn extract_refs<S: std::hash::BuildHasher>(
    source: &str,
    file_path: &str,
    known_symbols: &std::collections::HashSet<String, S>,
) -> Result<Vec<SymbolRef>, String> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .map_err(|e| format!("Failed to set language: {e}"))?;

    let tree = parser.parse(source, None).ok_or("Failed to parse source")?;
    let root = tree.root_node();
    let mut refs = Vec::new();
    collect_refs(&root, source, file_path, known_symbols, &mut refs);
    Ok(refs)
}

fn collect_refs<S: std::hash::BuildHasher>(
    node: &Node,
    source: &str,
    file_path: &str,
    known_symbols: &std::collections::HashSet<String, S>,
    refs: &mut Vec<SymbolRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_item" => {
                let Some(name_node) = find_child_by_kind(&child, "identifier") else {
                    continue;
                };
                let fn_name = node_text(&name_node, source);
                collect_body_refs(&child, source, file_path, &fn_name, known_symbols, refs);
            }
            "impl_item" | "declaration_list" => {
                collect_refs(&child, source, file_path, known_symbols, refs);
            }
            _ => {}
        }
    }
}

fn collect_body_refs<S: std::hash::BuildHasher>(
    fn_node: &Node,
    source: &str,
    file_path: &str,
    fn_name: &str,
    known_symbols: &std::collections::HashSet<String, S>,
    refs: &mut Vec<SymbolRef>,
) {
    let mut seen = std::collections::HashSet::new();
    let mut stack = vec![*fn_node];

    while let Some(node) = stack.pop() {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "type_identifier" => {
                    let name = node_text(&child, source);
                    if name != fn_name && known_symbols.contains(&name) && seen.insert(name.clone())
                    {
                        refs.push(SymbolRef {
                            source_name: fn_name.to_string(),
                            source_file: file_path.to_string(),
                            target_name: name,
                            kind: RefKind::TypeRef,
                        });
                    }
                }
                "identifier" => {
                    let name = node_text(&child, source);
                    if name != fn_name && known_symbols.contains(&name) && seen.insert(name.clone())
                    {
                        refs.push(SymbolRef {
                            source_name: fn_name.to_string(),
                            source_file: file_path.to_string(),
                            target_name: name,
                            kind: RefKind::Call,
                        });
                    }
                }
                _ => {
                    stack.push(child);
                }
            }
        }
    }
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;

    #[test]
    fn test_extract_function() {
        let source = r#"
pub fn hello(name: &str) -> String {
    format!("Hello, {name}")
}
"#;
        let symbols = parse_rust_source(source, "src/lib.rs").unwrap();
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "hello");
        assert_eq!(symbols[0].kind, SymbolKind::Function);
        assert_eq!(symbols[0].visibility, Visibility::Public);
    }

    #[test]
    fn test_extract_struct_and_impl() {
        let source = r"
pub struct Config {
    pub port: u16,
}

impl Config {
    pub fn new(port: u16) -> Self {
        Self { port }
    }
}
";
        let symbols = parse_rust_source(source, "src/config.rs").unwrap();
        let struct_sym = symbols
            .iter()
            .find(|s| s.name == "Config" && s.kind == SymbolKind::Struct)
            .unwrap();
        assert_eq!(struct_sym.kind, SymbolKind::Struct);
        let method = symbols.iter().find(|s| s.name == "new").unwrap();
        assert_eq!(method.kind, SymbolKind::Function);
    }

    #[test]
    fn test_extract_use_statements() {
        let source = r"
use std::collections::HashMap;
use serde::{Serialize, Deserialize};
";
        let symbols = parse_rust_source(source, "src/lib.rs").unwrap();
        let uses: Vec<_> = symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Use)
            .collect();
        assert_eq!(uses.len(), 2);
    }

    #[test]
    fn test_extract_enum_and_trait() {
        let source = r"
pub enum Color { Red, Green, Blue }
pub trait Drawable { fn draw(&self); }
";
        let symbols = parse_rust_source(source, "src/lib.rs").unwrap();
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "Color" && s.kind == SymbolKind::Enum)
        );
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "Drawable" && s.kind == SymbolKind::Trait)
        );
    }

    #[test]
    fn test_private_function() {
        let source = r"
fn private_helper() {}
";
        let symbols = parse_rust_source(source, "src/lib.rs").unwrap();
        assert_eq!(symbols[0].visibility, Visibility::Private);
    }

    #[test]
    fn test_extract_refs_type_usage() {
        let source = r"
pub struct Config { pub port: u16 }

pub fn create_config() -> Config {
    Config { port: 8080 }
}
";
        let known: std::collections::HashSet<String> = ["Config", "create_config"]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        let refs = extract_refs(source, "src/lib.rs", &known).unwrap();
        let config_ref = refs.iter().find(|r| r.target_name == "Config");
        assert!(config_ref.is_some(), "should find Config reference");
        assert_eq!(config_ref.unwrap().source_name, "create_config");
    }

    #[test]
    fn test_extract_refs_function_call() {
        let source = r"
fn helper() -> i32 { 42 }

pub fn caller() -> i32 {
    helper()
}
";
        let known: std::collections::HashSet<String> = ["helper", "caller"]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        let refs = extract_refs(source, "src/lib.rs", &known).unwrap();
        let helper_ref = refs.iter().find(|r| r.target_name == "helper");
        assert!(helper_ref.is_some(), "should find helper call");
        assert_eq!(helper_ref.unwrap().source_name, "caller");
    }

    #[test]
    fn test_extract_refs_no_self_ref() {
        let source = r"
pub fn standalone() -> i32 { 42 }
";
        let known: std::collections::HashSet<String> =
            ["standalone"].iter().map(|s| (*s).to_string()).collect();
        let refs = extract_refs(source, "src/lib.rs", &known).unwrap();
        assert!(refs.is_empty(), "should not create self-references");
    }
}
