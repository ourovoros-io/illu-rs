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
    Const,
    Static,
    TypeAlias,
    Macro,
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
            Self::Const => "const",
            Self::Static => "static",
            Self::TypeAlias => "type_alias",
            Self::Macro => "macro",
        };
        f.write_str(s)
    }
}

impl std::str::FromStr for SymbolKind {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "function" => Ok(Self::Function),
            "struct" => Ok(Self::Struct),
            "enum" => Ok(Self::Enum),
            "trait" => Ok(Self::Trait),
            "impl" => Ok(Self::Impl),
            "use" => Ok(Self::Use),
            "mod" => Ok(Self::Mod),
            "const" => Ok(Self::Const),
            "static" => Ok(Self::Static),
            "type_alias" => Ok(Self::TypeAlias),
            "macro" => Ok(Self::Macro),
            other => Err(format!("unknown symbol kind: {other}")),
        }
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

impl std::str::FromStr for Visibility {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "public" => Ok(Self::Public),
            "pub(crate)" => Ok(Self::PublicCrate),
            "private" => Ok(Self::Private),
            other => Err(format!("unknown visibility: {other}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    pub visibility: Visibility,
    pub file_path: String,
    pub line_start: usize,
    pub line_end: usize,
    pub signature: String,
    pub doc_comment: Option<String>,
    pub body: Option<String>,
    pub details: Option<String>,
}

pub fn parse_rust_source(
    source: &str,
    file_path: &str,
) -> Result<(Vec<Symbol>, Vec<TraitImpl>), String> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .map_err(|e| format!("Failed to set language: {e}"))?;

    let tree = parser.parse(source, None).ok_or("Failed to parse source")?;

    let root = tree.root_node();
    let mut symbols = Vec::new();
    let mut trait_impls = Vec::new();
    extract_symbols(&root, source, file_path, &mut symbols, &mut trait_impls);
    Ok((symbols, trait_impls))
}

fn extract_doc_comment(node: &Node, source: &str) -> Option<String> {
    let mut lines = Vec::new();
    let mut sibling = node.prev_sibling();
    while let Some(sib) = sibling {
        match sib.kind() {
            "line_comment" => {
                let text = node_text(&sib, source);
                if let Some(stripped) = text.strip_prefix("///") {
                    let stripped = stripped.strip_prefix(' ').unwrap_or(stripped);
                    lines.push(stripped.trim_end().to_string());
                } else {
                    break;
                }
            }
            "block_comment" => {
                let text = node_text(&sib, source);
                if let Some(inner) = text.strip_prefix("/**") {
                    let inner = inner.strip_suffix("*/").unwrap_or(inner);
                    lines.push(inner.trim().to_string());
                } else {
                    break;
                }
            }
            "attribute_item" => {
                // Skip attributes like #[derive(...)]
            }
            _ => break,
        }
        sibling = sib.prev_sibling();
    }
    if lines.is_empty() {
        return None;
    }
    lines.reverse();
    Some(lines.join("\n"))
}

fn extract_body(node: &Node, source: &str) -> String {
    let text = &source[node.start_byte()..node.end_byte()];
    let line_count = text.lines().count();
    if line_count > 100 {
        let truncated: String = text.lines().take(100).collect::<Vec<_>>().join("\n");
        format!("{truncated}\n// ... truncated")
    } else {
        text.to_string()
    }
}

fn extract_symbols(
    node: &Node,
    source: &str,
    file_path: &str,
    symbols: &mut Vec<Symbol>,
    trait_impls: &mut Vec<TraitImpl>,
) {
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
                if let Some(ti) = extract_trait_impl_info(&child, source, file_path) {
                    trait_impls.push(ti);
                }
                symbols.push(Symbol {
                    name: type_name.clone().unwrap_or_default(),
                    kind: SymbolKind::Impl,
                    visibility: vis,
                    file_path: file_path.to_string(),
                    line_start: child.start_position().row + 1,
                    line_end: child.end_position().row + 1,
                    signature: sig,
                    doc_comment: None,
                    body: Some(extract_body(&child, source)),
                    details: None,
                });
                extract_symbols(&child, source, file_path, symbols, trait_impls);
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
                    doc_comment: None,
                    body: None,
                    details: None,
                });
            }
            "mod_item" => {
                if let Some(sym) = extract_named_item(&child, source, file_path, SymbolKind::Mod) {
                    symbols.push(sym);
                }
            }
            "const_item" => {
                if let Some(sym) =
                    extract_named_item(&child, source, file_path, SymbolKind::Const)
                {
                    symbols.push(sym);
                }
            }
            "static_item" => {
                if let Some(sym) =
                    extract_named_item(&child, source, file_path, SymbolKind::Static)
                {
                    symbols.push(sym);
                }
            }
            "type_item" => {
                if let Some(sym) =
                    extract_named_item(&child, source, file_path, SymbolKind::TypeAlias)
                {
                    symbols.push(sym);
                }
            }
            "macro_definition" => {
                if let Some(sym) = extract_macro_def(&child, source, file_path) {
                    symbols.push(sym);
                }
            }
            "declaration_list" => {
                extract_symbols(&child, source, file_path, symbols, trait_impls);
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
        doc_comment: extract_doc_comment(node, source),
        body: Some(extract_body(node, source)),
        details: None,
    })
}

fn extract_struct_details(node: &Node, source: &str) -> Option<String> {
    let list = find_child_by_kind(node, "field_declaration_list")?;
    let mut fields = Vec::new();
    let mut cursor = list.walk();
    for child in list.children(&mut cursor) {
        if child.kind() == "field_declaration" {
            let text = node_text(&child, source).trim_end().to_string();
            let text = text.strip_suffix(',').unwrap_or(&text).to_string();
            fields.push(text);
        }
    }
    if fields.is_empty() {
        return None;
    }
    Some(fields.join("\n"))
}

fn extract_enum_details(node: &Node, source: &str) -> Option<String> {
    let list = find_child_by_kind(node, "enum_variant_list")?;
    let mut variants = Vec::new();
    let mut cursor = list.walk();
    for child in list.children(&mut cursor) {
        if child.kind() == "enum_variant" {
            let text = node_text(&child, source).trim_end().to_string();
            let text = text.strip_suffix(',').unwrap_or(&text).to_string();
            variants.push(text);
        }
    }
    if variants.is_empty() {
        return None;
    }
    Some(variants.join("\n"))
}

fn extract_trait_details(node: &Node, source: &str) -> Option<String> {
    let list = find_child_by_kind(node, "declaration_list")?;
    let mut methods = Vec::new();
    let mut cursor = list.walk();
    for child in list.children(&mut cursor) {
        if child.kind() == "function_signature_item" || child.kind() == "function_item" {
            let sig = get_first_line(&child, source);
            let sig = sig.strip_suffix(';').unwrap_or(&sig).to_string();
            methods.push(sig);
        }
    }
    if methods.is_empty() {
        return None;
    }
    Some(methods.join("\n"))
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

    let details = match kind {
        SymbolKind::Struct => extract_struct_details(node, source),
        SymbolKind::Enum => extract_enum_details(node, source),
        SymbolKind::Trait => extract_trait_details(node, source),
        _ => None,
    };

    Some(Symbol {
        name,
        kind,
        visibility: vis,
        file_path: file_path.to_string(),
        line_start: node.start_position().row + 1,
        line_end: node.end_position().row + 1,
        signature: sig,
        doc_comment: extract_doc_comment(node, source),
        body: Some(extract_body(node, source)),
        details,
    })
}

fn extract_macro_def(node: &Node, source: &str, file_path: &str) -> Option<Symbol> {
    let name = find_child_by_kind(node, "identifier")?;
    let name_text = node_text(&name, source);
    let sig = get_first_line(node, source);

    Some(Symbol {
        name: name_text,
        kind: SymbolKind::Macro,
        visibility: Visibility::Public,
        file_path: file_path.to_string(),
        line_start: node.start_position().row + 1,
        line_end: node.end_position().row + 1,
        signature: sig,
        doc_comment: extract_doc_comment(node, source),
        body: Some(extract_body(node, source)),
        details: None,
    })
}

fn extract_impl_type(node: &Node, source: &str) -> Option<String> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .find(|child| child.kind() == "type_identifier" || child.kind() == "generic_type")
        .map(|child| node_text(&child, source))
}

fn is_type_node(kind: &str) -> bool {
    kind == "type_identifier" || kind == "scoped_type_identifier" || kind == "generic_type"
}

fn extract_type_name(node: &Node, source: &str) -> String {
    if node.kind() == "generic_type"
        && let Some(base) = find_child_by_kind(node, "type_identifier")
    {
        return node_text(&base, source);
    }
    node_text(node, source)
}

fn extract_trait_impl_info(node: &Node, source: &str, file_path: &str) -> Option<TraitImpl> {
    let mut cursor = node.walk();
    let children: Vec<_> = node.children(&mut cursor).collect();

    let for_pos = children.iter().position(|c| c.kind() == "for")?;

    let trait_node = children[..for_pos]
        .iter()
        .rfind(|c| is_type_node(c.kind()))?;

    let type_node = children[for_pos + 1..]
        .iter()
        .find(|c| is_type_node(c.kind()))?;

    Some(TraitImpl {
        trait_name: extract_type_name(trait_node, source),
        type_name: extract_type_name(type_node, source),
        file_path: file_path.to_string(),
        line_start: node.start_position().row + 1,
        line_end: node.end_position().row + 1,
    })
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraitImpl {
    pub type_name: String,
    pub trait_name: String,
    pub file_path: String,
    pub line_start: usize,
    pub line_end: usize,
}

const NOISY_SYMBOL_NAMES: &[&str] = &[
    "new", "default", "from", "into", "clone", "fmt", "eq", "ne",
    "partial_cmp", "cmp", "hash", "drop", "deref", "deref_mut",
    "as_ref", "as_mut", "borrow", "borrow_mut", "to_string",
    "to_owned", "try_from", "try_into", "build", "init",
];

fn is_noisy_symbol(name: &str) -> bool {
    NOISY_SYMBOL_NAMES.contains(&name)
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
                    if name != fn_name
                        && !is_noisy_symbol(&name)
                        && known_symbols.contains(&name)
                        && seen.insert(name.clone())
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
                    if name != fn_name
                        && !is_noisy_symbol(&name)
                        && known_symbols.contains(&name)
                        && seen.insert(name.clone())
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
        let (symbols, _) = parse_rust_source(source, "src/lib.rs").unwrap();
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
        let (symbols, _) = parse_rust_source(source, "src/config.rs").unwrap();
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
        let (symbols, _) = parse_rust_source(source, "src/lib.rs").unwrap();
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
        let (symbols, _) = parse_rust_source(source, "src/lib.rs").unwrap();
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
        let (symbols, _) = parse_rust_source(source, "src/lib.rs").unwrap();
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

    #[test]
    fn test_extract_doc_comments() {
        let source = r"
/// First line of docs
/// Second line of docs
pub fn documented() {}

pub fn undocumented() {}

/// Doc comment here
#[derive(Debug)]
pub struct Annotated;
";
        let (symbols, _) = parse_rust_source(source, "src/lib.rs").unwrap();

        let documented = symbols.iter().find(|s| s.name == "documented").unwrap();
        let doc = documented.doc_comment.as_deref().unwrap();
        assert!(doc.contains("First line of docs"));
        assert!(doc.contains("Second line of docs"));
        assert_eq!(doc.lines().count(), 2);

        let undocumented = symbols.iter().find(|s| s.name == "undocumented").unwrap();
        assert!(undocumented.doc_comment.is_none());

        let annotated = symbols.iter().find(|s| s.name == "Annotated").unwrap();
        let doc = annotated.doc_comment.as_deref().unwrap();
        assert!(doc.contains("Doc comment here"));
    }

    #[test]
    fn test_extract_body() {
        let source = r#"
pub fn greet(name: &str) -> String {
    format!("Hello, {name}")
}
"#;
        let (symbols, _) = parse_rust_source(source, "src/lib.rs").unwrap();
        let sym = symbols.iter().find(|s| s.name == "greet").unwrap();
        let body = sym.body.as_deref().unwrap();
        assert!(body.contains("pub fn greet"));
        assert!(body.contains("format!"));
    }

    #[test]
    fn test_body_truncation() {
        use std::fmt::Write;
        let mut source = String::from("pub fn long_fn() {\n");
        for i in 0..110 {
            let _ = writeln!(source, "    let x{i} = {i};");
        }
        source.push_str("}\n");

        let (symbols, _) = parse_rust_source(&source, "src/lib.rs").unwrap();
        let sym = symbols.iter().find(|s| s.name == "long_fn").unwrap();
        let body = sym.body.as_deref().unwrap();
        assert!(body.contains("// ... truncated"));
        assert!(body.lines().count() <= 101);
    }

    #[test]
    fn test_extract_struct_fields() {
        let source = r"
pub struct Config {
    pub port: u16,
    pub host: String,
}
";
        let (symbols, _) = parse_rust_source(source, "src/lib.rs").unwrap();
        let sym = symbols.iter().find(|s| s.name == "Config").unwrap();
        let details = sym.details.as_deref().unwrap();
        assert!(details.contains("pub port: u16"));
        assert!(details.contains("pub host: String"));
        assert_eq!(details.lines().count(), 2);
    }

    #[test]
    fn test_extract_enum_variants() {
        let source = r"
pub enum Color {
    Red,
    Green(u8),
    Blue { intensity: f32 },
}
";
        let (symbols, _) = parse_rust_source(source, "src/lib.rs").unwrap();
        let sym = symbols.iter().find(|s| s.name == "Color").unwrap();
        let details = sym.details.as_deref().unwrap();
        assert!(details.contains("Red"));
        assert!(details.contains("Green(u8)"));
        assert!(details.contains("Blue { intensity: f32 }"));
    }

    #[test]
    fn test_extract_trait_methods() {
        let source = r"
pub trait Drawable {
    fn draw(&self);
    fn resize(&mut self, width: u32, height: u32);
}
";
        let (symbols, _) = parse_rust_source(source, "src/lib.rs").unwrap();
        let sym = symbols.iter().find(|s| s.name == "Drawable").unwrap();
        let details = sym.details.as_deref().unwrap();
        assert!(details.contains("fn draw(&self)"));
        assert!(details.contains("fn resize(&mut self, width: u32, height: u32)"));
    }

    #[test]
    fn test_unit_struct_no_details() {
        let source = r"
pub struct Empty;
";
        let (symbols, _) = parse_rust_source(source, "src/lib.rs").unwrap();
        let sym = symbols.iter().find(|s| s.name == "Empty").unwrap();
        assert!(sym.details.is_none());
    }

    #[test]
    fn test_extract_trait_impl() {
        let source = r"
pub trait Drawable {
    fn draw(&self);
}

pub struct Circle;

impl Drawable for Circle {
    fn draw(&self) {}
}
";
        let (_, trait_impls) = parse_rust_source(source, "src/lib.rs").unwrap();
        assert_eq!(trait_impls.len(), 1);
        assert_eq!(trait_impls[0].trait_name, "Drawable");
        assert_eq!(trait_impls[0].type_name, "Circle");
        assert_eq!(trait_impls[0].file_path, "src/lib.rs");
    }

    #[test]
    fn test_inherent_impl_not_trait_impl() {
        let source = r"
pub struct Foo;

impl Foo {
    pub fn new() -> Self { Self }
}
";
        let (_, trait_impls) = parse_rust_source(source, "src/lib.rs").unwrap();
        assert!(trait_impls.is_empty());
    }

    #[test]
    fn test_extract_refs_filters_noisy_names() {
        let source = r"
pub struct Config {
    pub port: u16,
}

impl Config {
    pub fn new(port: u16) -> Self {
        Self { port }
    }
}

pub fn caller() -> Config {
    Config::new(8080)
}
";
        let known: std::collections::HashSet<String> = ["Config", "new", "caller"]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        let refs = extract_refs(source, "src/lib.rs", &known).unwrap();
        assert!(
            refs.iter().any(|r| r.target_name == "Config"),
            "should find Config as a target"
        );
        assert!(
            !refs.iter().any(|r| r.target_name == "new"),
            "should NOT find 'new' as a target (noisy symbol)"
        );
    }

    #[test]
    fn test_extract_const() {
        let source = r"
/// Maximum retry count.
pub const MAX_RETRIES: u32 = 3;
";
        let (symbols, _) = parse_rust_source(source, "src/lib.rs").unwrap();
        let sym = symbols.iter().find(|s| s.name == "MAX_RETRIES");
        assert!(sym.is_some(), "should extract const");
        let sym = sym.unwrap();
        assert_eq!(sym.kind, SymbolKind::Const);
        assert_eq!(sym.visibility, Visibility::Public);
        assert!(sym.doc_comment.as_deref().unwrap().contains("Maximum"));
    }

    #[test]
    fn test_extract_static() {
        let source = r"
pub static GLOBAL_COUNT: u64 = 0;
";
        let (symbols, _) = parse_rust_source(source, "src/lib.rs").unwrap();
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "GLOBAL_COUNT" && s.kind == SymbolKind::Static)
        );
    }

    #[test]
    fn test_extract_type_alias() {
        let source = r"
pub type Result<T> = std::result::Result<T, MyError>;
";
        let (symbols, _) = parse_rust_source(source, "src/lib.rs").unwrap();
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "Result" && s.kind == SymbolKind::TypeAlias)
        );
    }

    #[test]
    fn test_extract_macro_rules() {
        let source = r"
/// Helper macro for creating responses.
macro_rules! response {
    ($code:expr, $body:expr) => {
        Response::new($code, $body)
    };
}
";
        let (symbols, _) = parse_rust_source(source, "src/lib.rs").unwrap();
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "response" && s.kind == SymbolKind::Macro)
        );
    }

    #[test]
    fn test_generic_trait_impl() {
        let source = r#"
use std::fmt;

pub struct MyType;

impl fmt::Display for MyType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "MyType")
    }
}
"#;
        let (_, trait_impls) = parse_rust_source(source, "src/lib.rs").unwrap();
        assert_eq!(trait_impls.len(), 1);
        assert_eq!(trait_impls[0].type_name, "MyType");
        // scoped_type_identifier gives full path
        assert!(
            trait_impls[0].trait_name.contains("Display"),
            "trait_name was: {}",
            trait_impls[0].trait_name
        );
    }
}
