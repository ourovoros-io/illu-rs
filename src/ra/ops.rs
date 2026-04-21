//! Composed high-level operations combining multiple LSP calls.

use std::collections::HashMap;

use lsp_types::{TextEdit, WorkspaceEdit};

use super::client::RaClient;
use super::error::{RaError, Result};
use super::types::{
    FileReferences, PositionSpec, RenameImpact, RenameResult, RichLocation, SymbolContext,
    url_to_path_string,
};

// ── Symbol Context ──────────────────────────────────────────────────────

impl RaClient {
    /// Get full context for a symbol: definition, hover, callers, callees,
    /// implementations, and related tests.
    pub async fn symbol_context(&self, spec: &PositionSpec) -> Result<SymbolContext> {
        let definitions = self.definition(spec).await?;
        let def_location = definitions.first().map_or_else(
            || RichLocation {
                file: spec.file.display().to_string(),
                line: spec.line,
                col: spec.col,
                end_line: spec.line,
                end_col: spec.col,
                context_before: None,
                text: String::new(),
                context_after: None,
            },
            RichLocation::from_location,
        );

        let def_location = tokio::task::spawn_blocking(move || enrich_location(def_location))
            .await
            .map_err(|e| RaError::RequestFailed(format!("enrich_location task failed: {e}")))?;

        let hover = self.hover(spec).await.unwrap_or(None);
        let refs = self.references(spec, false).await.unwrap_or_default();
        let reference_count = refs.len();

        let (incoming_calls, outgoing_calls) = match self.prepare_call_hierarchy(spec).await {
            Ok(items) if !items.is_empty() => {
                // Safe: we just checked !items.is_empty(), so next() always returns Some
                let Some(item) = items.into_iter().next() else {
                    return Ok(SymbolContext {
                        name: String::new(),
                        hover: None,
                        definition: def_location,
                        reference_count: 0,
                        incoming_calls: vec![],
                        outgoing_calls: vec![],
                        implementations: vec![],
                        related_tests: vec![],
                    });
                };
                let incoming = self.incoming_calls(item.clone()).await.unwrap_or_default();
                let outgoing = self.outgoing_calls(item).await.unwrap_or_default();
                (incoming, outgoing)
            }
            _ => (vec![], vec![]),
        };

        let implementations = self
            .implementation(spec)
            .await
            .unwrap_or_default()
            .iter()
            .map(RichLocation::from_location)
            .collect();

        let related_tests = self
            .related_tests(spec)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|t| t.runnable.label)
            .collect();

        let name = hover
            .as_ref()
            .and_then(|h| {
                h.lines()
                    .find(|l| {
                        l.contains("fn ")
                            || l.contains("struct ")
                            || l.contains("trait ")
                            || l.contains("enum ")
                            || l.contains("type ")
                            || l.contains("const ")
                            || l.contains("static ")
                            || l.contains("mod ")
                    })
                    .map(|l| l.trim().to_string())
            })
            .unwrap_or_else(|| {
                format!(
                    "symbol at {}:{}:{}",
                    spec.file.display(),
                    spec.line,
                    spec.col
                )
            });

        Ok(SymbolContext {
            name,
            hover,
            definition: def_location,
            reference_count,
            incoming_calls,
            outgoing_calls,
            implementations,
            related_tests,
        })
    }
}

/// Add source text context to a `RichLocation` by reading from disk.
fn enrich_location(mut loc: RichLocation) -> RichLocation {
    if let Ok(path) = std::path::Path::new(&loc.file).canonicalize()
        && let Ok((before, text, after)) = super::types::read_context_lines(&path, loc.line, 2)
    {
        loc.context_before = Some(before);
        loc.text = text;
        loc.context_after = Some(after);
    }
    loc
}

// ── Safe Rename ─────────────────────────────────────────────────────────

impl RaClient {
    /// Preview the impact of a rename without applying it.
    pub async fn rename_preview(
        &self,
        spec: &PositionSpec,
        new_name: &str,
    ) -> Result<RenameImpact> {
        let Some(prepare) = self.prepare_rename(spec).await? else {
            return Err(RaError::RequestFailed(
                "rename not available at this position".to_string(),
            ));
        };

        let old_name = match &prepare {
            lsp_types::PrepareRenameResponse::RangeWithPlaceholder { placeholder, .. } => {
                placeholder.clone()
            }
            lsp_types::PrepareRenameResponse::Range(range) => {
                if let Ok(content) = std::fs::read_to_string(&spec.file) {
                    let lines: Vec<&str> = content.lines().collect();
                    if let Some(line) = lines.get(range.start.line as usize) {
                        let start = range.start.character as usize;
                        let end = range.end.character as usize;
                        if start <= end {
                            let text: String = line.chars().skip(start).take(end - start).collect();
                            if text.is_empty() {
                                "unknown".to_string()
                            } else {
                                text
                            }
                        } else {
                            "unknown".to_string()
                        }
                    } else {
                        "unknown".to_string()
                    }
                } else {
                    "unknown".to_string()
                }
            }
            lsp_types::PrepareRenameResponse::DefaultBehavior { .. } => "unknown".to_string(),
        };

        let refs = self.references(spec, true).await.unwrap_or_default();

        let mut by_file: HashMap<String, Vec<u32>> = HashMap::new();
        for loc in &refs {
            let file = url_to_path_string(&loc.uri);
            by_file
                .entry(file)
                .or_default()
                .push(loc.range.start.line + 1);
        }

        let files_affected: Vec<String> = by_file.keys().cloned().collect();
        let references_by_file: Vec<FileReferences> = by_file
            .into_iter()
            .map(|(file, lines)| FileReferences {
                file,
                count: lines.len(),
                lines,
            })
            .collect();

        Ok(RenameImpact {
            old_name,
            new_name: new_name.to_string(),
            files_affected,
            total_references: refs.len(),
            references_by_file,
        })
    }

    /// Perform a safe rename: preview, apply, and check for new diagnostics.
    pub async fn safe_rename(&self, spec: &PositionSpec, new_name: &str) -> Result<RenameResult> {
        let preview = self.rename_preview(spec, new_name).await?;

        let Some(edit) = self.rename(spec, new_name).await? else {
            return Err(RaError::RequestFailed(
                "rename returned no edits".to_string(),
            ));
        };

        let total_edits = count_edits(&edit);
        let files_changed = tokio::task::spawn_blocking(move || apply_workspace_edit(&edit))
            .await
            .map_err(|e| {
                RaError::RequestFailed(format!("apply_workspace_edit task failed: {e}"))
            })??;

        for file in &files_changed {
            let path = std::path::Path::new(file);
            if path.exists() {
                let _ = self.ensure_open(path).await;
            }
        }

        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        let mut new_diagnostics = vec![];
        for file in &files_changed {
            let path = std::path::Path::new(file);
            let diags = self.get_diagnostics(path).unwrap_or_default();
            new_diagnostics.extend(diags);
        }

        Ok(RenameResult {
            old_name: preview.old_name,
            new_name: new_name.to_string(),
            files_changed,
            total_edits,
            new_diagnostics,
        })
    }
}

// ── Workspace Edit Application ──────────────────────────────────────────

/// Apply a `WorkspaceEdit` to files on disk. Returns the list of changed files.
pub fn apply_workspace_edit(edit: &WorkspaceEdit) -> Result<Vec<String>> {
    let mut changed_files = vec![];

    if let Some(changes) = &edit.changes {
        for (uri, edits) in changes {
            let path = uri
                .to_file_path()
                .map_err(|()| RaError::RequestFailed(format!("invalid URI: {uri}")))?;
            apply_text_edits(&path, edits)?;
            changed_files.push(path.display().to_string());
        }
    }

    if let Some(doc_changes) = &edit.document_changes {
        match doc_changes {
            lsp_types::DocumentChanges::Edits(edits) => {
                for edit in edits {
                    let path = edit.text_document.uri.to_file_path().map_err(|()| {
                        RaError::RequestFailed(format!("invalid URI: {}", edit.text_document.uri))
                    })?;
                    let text_edits: Vec<_> = edit
                        .edits
                        .iter()
                        .map(|e| match e {
                            lsp_types::OneOf::Left(te) => te.clone(),
                            lsp_types::OneOf::Right(ate) => ate.text_edit.clone(),
                        })
                        .collect();
                    apply_text_edits(&path, &text_edits)?;
                    changed_files.push(path.display().to_string());
                }
            }
            lsp_types::DocumentChanges::Operations(ops) => {
                for op in ops {
                    if let lsp_types::DocumentChangeOperation::Edit(edit) = op {
                        let path = edit.text_document.uri.to_file_path().map_err(|()| {
                            RaError::RequestFailed(format!(
                                "invalid URI: {}",
                                edit.text_document.uri
                            ))
                        })?;
                        let text_edits: Vec<_> = edit
                            .edits
                            .iter()
                            .map(|e| match e {
                                lsp_types::OneOf::Left(te) => te.clone(),
                                lsp_types::OneOf::Right(ate) => ate.text_edit.clone(),
                            })
                            .collect();
                        apply_text_edits(&path, &text_edits)?;
                        changed_files.push(path.display().to_string());
                    }
                }
            }
        }
    }

    Ok(changed_files)
}

fn apply_text_edits(path: &std::path::Path, edits: &[TextEdit]) -> Result<()> {
    let content = std::fs::read_to_string(path)?;
    let lines: Vec<&str> = content.lines().collect();

    let mut indexed_edits: Vec<(usize, usize, &str)> = edits
        .iter()
        .filter_map(|edit| {
            let start = position_to_offset(&lines, &content, edit.range.start)?;
            let end = position_to_offset(&lines, &content, edit.range.end)?;
            Some((start, end, edit.new_text.as_str()))
        })
        .collect();

    indexed_edits.sort_by_key(|v| std::cmp::Reverse(v.0));

    let mut result = content;
    for (start, end, new_text) in indexed_edits {
        if start <= result.len() && end <= result.len() {
            result.replace_range(start..end, new_text);
        }
    }

    std::fs::write(path, result)?;
    Ok(())
}

fn position_to_offset(lines: &[&str], content: &str, pos: lsp_types::Position) -> Option<usize> {
    let line = pos.line as usize;
    let col = pos.character as usize;

    // Detect line ending style from the raw content
    let line_ending_len = if content.contains("\r\n") { 2 } else { 1 };

    let mut offset = 0;
    for (i, line_text) in lines.iter().enumerate() {
        if i == line {
            let byte_col = utf16_to_byte_offset(line_text, col);
            return Some(offset + byte_col);
        }
        offset += line_text.len() + line_ending_len;
    }

    if line == lines.len() && col == 0 {
        Some(content.len())
    } else {
        None
    }
}

fn utf16_to_byte_offset(text: &str, utf16_col: usize) -> usize {
    let mut utf16_count = 0;
    for (byte_idx, ch) in text.char_indices() {
        if utf16_count >= utf16_col {
            return byte_idx;
        }
        utf16_count += ch.len_utf16();
    }
    text.len()
}

fn count_edits(edit: &WorkspaceEdit) -> usize {
    let mut count = 0;
    if let Some(changes) = &edit.changes {
        for edits in changes.values() {
            count += edits.len();
        }
    }
    if let Some(doc_changes) = &edit.document_changes {
        match doc_changes {
            lsp_types::DocumentChanges::Edits(edits) => {
                for edit in edits {
                    count += edit.edits.len();
                }
            }
            lsp_types::DocumentChanges::Operations(ops) => {
                for op in ops {
                    if let lsp_types::DocumentChangeOperation::Edit(edit) = op {
                        count += edit.edits.len();
                    }
                }
            }
        }
    }
    count
}
