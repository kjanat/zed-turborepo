//! Turbo LSP Server
//!
//! Local-first Language Server Protocol implementation for Turborepo.

use std::{
    path::{Path, PathBuf},
    sync::Mutex,
};

use indexmap::{IndexMap, IndexSet};
use jsonc_parser::{
    CollectOptions, ParseOptions,
    ast::{ObjectPropName, StringLit},
    common::Range as JsonRange,
    parse_to_ast,
};
use tokio::io;
use tower_lsp::{
    Client, LspService, Server,
    lsp_types::{
        CodeAction, CodeActionKind, CodeActionOrCommand, CodeActionParams,
        CodeActionProviderCapability, CodeActionResponse, CodeLens, CodeLensOptions,
        CodeLensParams, Command, CompletionItem, CompletionItemKind, CompletionOptions,
        CompletionParams, CompletionResponse, Diagnostic, DiagnosticSeverity, DiagnosticTag,
        DidChangeConfigurationParams, DidChangeTextDocumentParams, DidChangeWatchedFilesParams,
        DidChangeWorkspaceFoldersParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams,
        DidSaveTextDocumentParams, ExecuteCommandOptions, ExecuteCommandParams,
        GotoDefinitionParams, GotoDefinitionResponse, Hover, HoverContents, HoverParams,
        InitializeParams, InitializeResult, InitializedParams, Location, MarkupContent, MarkupKind,
        NumberOrString, OneOf, OptionalVersionedTextDocumentIdentifier, Position, Range,
        ReferenceParams, ReferencesOptions, ServerCapabilities, ServerInfo,
        TextDocumentContentChangeEvent, TextDocumentEdit, TextDocumentSyncCapability,
        TextDocumentSyncKind, TextEdit, Url, WorkDoneProgressOptions, WorkspaceEdit,
        WorkspaceFoldersServerCapabilities, WorkspaceServerCapabilities,
    },
};
use turbo_core::{Package, PackageDiscovery, TurboConfig};

include!(concat!(env!("OUT_DIR"), "/doc_links_generated.rs"));

const ROOT_PACKAGE_NAME: &str = "//";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TaskReference<'a> {
    package: Option<&'a str>,
    task: &'a str,
}

impl<'a> TaskReference<'a> {
    fn parse(value: &'a str) -> Self {
        if let Some(task) = value.strip_prefix("//#") {
            return Self {
                package: Some(ROOT_PACKAGE_NAME),
                task,
            };
        }

        value.split_once('#').map_or(
            Self {
                package: None,
                task: value,
            },
            |(package, task)| Self {
                package: Some(package),
                task,
            },
        )
    }
}

struct TurboBackend {
    client: Client,
    repo_root: Mutex<Option<PathBuf>>,
    files: Mutex<IndexMap<Url, String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum HoverTarget {
    TopLevelKey(String),
    TaskName(String),
    TaskField {
        task_name: String,
        field_name: String,
    },
    DependsOnEntry {
        task_name: String,
        entry: String,
    },
}

#[derive(Debug, Clone)]
struct WorkspaceState {
    packages: Vec<Package>,
    task_packages: IndexMap<String, Vec<String>>,
    package_names: IndexSet<String>,
}

impl TurboBackend {
    fn new(client: Client) -> Self {
        Self {
            client,
            repo_root: Mutex::new(None),
            files: Mutex::new(IndexMap::new()),
        }
    }

    fn remember_root(&self, params: &InitializeParams) {
        let path = params
            .root_uri
            .as_ref()
            .and_then(|uri| uri.to_file_path().ok());
        if let Ok(mut repo_root) = self.repo_root.lock() {
            *repo_root = path;
        }
    }

    fn repo_root(&self) -> Option<PathBuf> {
        self.repo_root.lock().ok()?.clone()
    }

    fn read_open_file(&self, uri: &Url) -> Option<String> {
        let files = self.files.lock().ok()?;
        files.get(uri).cloned()
    }

    fn remember_open_file(&self, params: &DidOpenTextDocumentParams) {
        if let Ok(mut files) = self.files.lock() {
            files.insert(
                params.text_document.uri.clone(),
                params.text_document.text.clone(),
            );
        }
    }

    fn remember_closed_file(&self, params: &DidCloseTextDocumentParams) {
        if let Ok(mut files) = self.files.lock() {
            files.shift_remove(&params.text_document.uri);
        }
    }

    fn remember_changed_file(&self, params: &DidChangeTextDocumentParams) {
        if let Ok(mut files) = self.files.lock()
            && let Some(text) = files.get_mut(&params.text_document.uri)
        {
            apply_content_changes(text, &params.content_changes);
        }
    }

    async fn workspace_state(&self) -> Option<WorkspaceState> {
        let root = self.repo_root()?;
        let discovery = PackageDiscovery::new(root.clone());
        let mut packages = discovery.discover_packages().await.ok()?;

        if let Some(root_package) = load_root_package(&root).await
            && !packages
                .iter()
                .any(|package| package.path == root_package.path)
        {
            packages.push(root_package);
        }

        let mut task_packages: IndexMap<String, Vec<String>> = IndexMap::new();
        let mut package_names = IndexSet::new();

        for package in &packages {
            let package_name = if package.path == root {
                ROOT_PACKAGE_NAME.to_string()
            } else {
                package.name.clone()
            };
            package_names.insert(package_name.clone());

            for script_name in package.scripts.keys() {
                task_packages
                    .entry(script_name.clone())
                    .or_default()
                    .push(package_name.clone());
            }
        }

        if let Ok(config) = TurboConfig::find_and_load(&root).await {
            for task_name in config.task_names() {
                task_packages.entry(task_name.to_string()).or_default();
            }
        }

        Some(WorkspaceState {
            packages,
            task_packages,
            package_names,
        })
    }

    async fn publish_diagnostics(&self, uri: Url, version: Option<i32>) {
        let Some(text) = self.read_open_file(&uri) else {
            return;
        };

        let diagnostics = self.collect_diagnostics(&text).await;
        self.client
            .publish_diagnostics(uri, diagnostics, version)
            .await;
    }

    async fn collect_diagnostics(&self, text: &str) -> Vec<Diagnostic> {
        let Some(workspace) = self.workspace_state().await else {
            return Vec::new();
        };

        let Ok(parse) = parse_to_ast(text, &CollectOptions::default(), &ParseOptions::default())
        else {
            return Vec::new();
        };

        let Some(root) = parse.value.as_ref().and_then(|value| value.as_object()) else {
            return Vec::new();
        };

        let rope = text;
        let mut diagnostics = Vec::new();

        for task_group_name in ["tasks", "pipeline"] {
            let Some(task_group) = root.get_object(task_group_name) else {
                continue;
            };

            for property in &task_group.properties {
                if let ObjectPropName::String(name) = &property.name {
                    report_invalid_packages_and_tasks(&workspace, rope, &mut diagnostics, name);
                }

                let Some(task_object) = property.value.as_object() else {
                    continue;
                };

                if let Some(depends_on) = task_object.get_array("dependsOn") {
                    for entry in &depends_on.elements {
                        let Some(string) = entry.as_string_lit().cloned() else {
                            continue;
                        };

                        let task_name = property.name.as_str();
                        let suffix = if let Some(stripped) = strip_lit_prefix(&string, "^") {
                            diagnostics.push(Diagnostic {
                                message: format!(
                                    "The '^' means run `{}` in dependency packages before `{task_name}`.",
                                    stripped.value
                                ),
                                range: byte_range_to_lsp_range(rope, collapse_string_range(string.range)),
                                severity: Some(DiagnosticSeverity::HINT),
                                tags: Some(vec![DiagnosticTag::UNNECESSARY]),
                                ..Diagnostic::default()
                            });
                            stripped
                        } else {
                            if string.value == task_name {
                                diagnostics.push(Diagnostic {
                                    message: "A task cannot depend on itself.".to_string(),
                                    range: byte_range_to_lsp_range(
                                        rope,
                                        collapse_string_range(string.range),
                                    ),
                                    severity: Some(DiagnosticSeverity::ERROR),
                                    code: Some(NumberOrString::String(
                                        "turbo:self-dependency".to_string(),
                                    )),
                                    ..Diagnostic::default()
                                });
                                continue;
                            }
                            string
                        };

                        let normalized = if let Some(stripped) = strip_lit_prefix(&suffix, "$") {
                            diagnostics.push(Diagnostic {
                                message: "The `$` syntax is deprecated. Remove `$` from the dependency entry.".to_string(),
                                range: byte_range_to_lsp_range(rope, collapse_string_range(suffix.range)),
                                severity: Some(DiagnosticSeverity::ERROR),
                                code: Some(NumberOrString::String("deprecated:env-var".to_string())),
                                ..Diagnostic::default()
                            });
                            stripped
                        } else {
                            suffix
                        };

                        report_invalid_packages_and_tasks(
                            &workspace,
                            rope,
                            &mut diagnostics,
                            &normalized,
                        );
                    }
                }
            }
        }

        diagnostics
    }

    async fn completions(&self) -> Option<Vec<CompletionItem>> {
        let workspace = self.workspace_state().await?;
        let mut seen = IndexSet::new();
        let mut items = Vec::new();

        for (task_name, package_names) in &workspace.task_packages {
            if seen.insert(task_name.clone()) {
                items.push(CompletionItem {
                    label: task_name.clone(),
                    kind: Some(CompletionItemKind::FIELD),
                    ..CompletionItem::default()
                });
            }

            for package_name in package_names {
                let label = format!("{package_name}#{task_name}");
                if seen.insert(label.clone()) {
                    items.push(CompletionItem {
                        label,
                        kind: Some(CompletionItemKind::FIELD),
                        ..CompletionItem::default()
                    });
                }
            }
        }

        Some(items)
    }

    async fn references(&self, params: &ReferenceParams) -> Option<Vec<Location>> {
        let text = self.read_open_file(&params.text_document_position.text_document.uri)?;
        let offset = utf16_position_to_byte_offset(&text, params.text_document_position.position)?;
        let target = hover_target_for_offset(&text, offset)?;
        let label = task_target_label(&target)?;
        self.script_locations_for_label(&label).await
    }

    async fn script_locations_for_label(&self, label: &str) -> Option<Vec<Location>> {
        let workspace = self.workspace_state().await?;
        let task_ref = TaskReference::parse(label);

        let mut locations = Vec::new();
        for package in workspace.packages {
            let package_name = if package.path == self.repo_root()? {
                ROOT_PACKAGE_NAME
            } else {
                package.name.as_str()
            };

            if let Some(filter) = task_ref.package
                && filter != package_name
            {
                continue;
            }

            if !package.scripts.contains_key(task_ref.task) {
                continue;
            }

            let Ok(content) = tokio::fs::read_to_string(&package.package_json_path).await else {
                continue;
            };

            if let Some(location) =
                script_location(&content, &package.package_json_path, task_ref.task)
            {
                locations.push(location);
            }
        }

        Some(locations)
    }

    async fn goto_definition(
        &self,
        params: &GotoDefinitionParams,
    ) -> Option<GotoDefinitionResponse> {
        let uri = &params.text_document_position_params.text_document.uri;
        let text = self.read_open_file(uri)?;
        let offset =
            utf16_position_to_byte_offset(&text, params.text_document_position_params.position)?;
        let target = hover_target_for_offset(&text, offset)?;
        let label = task_target_label(&target)?;

        if let Some(location) = task_definition_location(&text, uri, &label) {
            return Some(GotoDefinitionResponse::Scalar(location));
        }

        let script_locations = self.script_locations_for_label(&label).await?;
        match script_locations.as_slice() {
            [] => None,
            [location] => Some(GotoDefinitionResponse::Scalar(location.clone())),
            _ => Some(GotoDefinitionResponse::Array(script_locations)),
        }
    }

    fn code_lens(&self, uri: &Url) -> Option<Vec<CodeLens>> {
        let text = self.read_open_file(uri)?;
        let parse =
            parse_to_ast(&text, &CollectOptions::default(), &ParseOptions::default()).ok()?;
        let root = parse.value.as_ref()?.as_object()?;
        let mut items = Vec::new();

        for task_group_name in ["tasks", "pipeline"] {
            let Some(task_group) = root.get_object(task_group_name) else {
                continue;
            };

            for property in &task_group.properties {
                let range = key_range(property.range, property.name.as_str().len());
                items.push(CodeLens {
                    command: Some(Command {
                        title: format!("Run {}", property.name.as_str()),
                        command: "turbo.run".to_string(),
                        arguments: Some(vec![serde_json::Value::String(
                            property.name.as_str().to_string(),
                        )]),
                    }),
                    range: byte_range_to_lsp_range(&text, range),
                    data: None,
                });
            }
        }

        Some(items)
    }

    fn quickfixes(&self, params: &CodeActionParams) -> Vec<CodeActionOrCommand> {
        let Some(text) = self.read_open_file(&params.text_document.uri) else {
            return Vec::new();
        };

        let mut actions = Vec::new();
        for diagnostic in &params.context.diagnostics {
            let Some(NumberOrString::String(code)) = &diagnostic.code else {
                continue;
            };
            if code != "deprecated:env-var" {
                continue;
            }

            let Some(start) = utf16_position_to_byte_offset(&text, diagnostic.range.start) else {
                continue;
            };
            if start == 0 || text.as_bytes().get(start - 1) != Some(&b'$') {
                continue;
            }

            let edit = TextEdit {
                range: byte_range_to_lsp_range(&text, (start - 1)..start),
                new_text: String::new(),
            };
            let workspace_edit = WorkspaceEdit {
                changes: None,
                document_changes: Some(tower_lsp::lsp_types::DocumentChanges::Edits(vec![
                    TextDocumentEdit {
                        text_document: OptionalVersionedTextDocumentIdentifier {
                            uri: params.text_document.uri.clone(),
                            version: None,
                        },
                        edits: vec![OneOf::Left(edit)],
                    },
                ])),
                change_annotations: None,
            };

            actions.push(CodeActionOrCommand::CodeAction(CodeAction {
                title: "Remove deprecated `$` prefix".to_string(),
                kind: Some(CodeActionKind::QUICKFIX),
                is_preferred: Some(true),
                diagnostics: Some(vec![diagnostic.clone()]),
                edit: Some(workspace_edit),
                ..CodeAction::default()
            }));
        }

        actions
    }

    async fn hover_markdown(&self, params: &HoverParams) -> Option<Hover> {
        let text = self.read_open_file(&params.text_document_position_params.text_document.uri)?;
        let offset =
            utf16_position_to_byte_offset(&text, params.text_document_position_params.position)?;
        let target = hover_target_for_offset(&text, offset)?;
        let repo_root = self.repo_root();
        let markdown = build_hover_markdown(repo_root.as_deref(), &target).await;

        Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: markdown,
            }),
            range: None,
        })
    }
}

#[tower_lsp::async_trait]
impl tower_lsp::LanguageServer for TurboBackend {
    async fn initialize(
        &self,
        params: InitializeParams,
    ) -> tower_lsp::jsonrpc::Result<InitializeResult> {
        self.remember_root(&params);

        Ok(InitializeResult {
            server_info: Some(ServerInfo {
                name: env!("CARGO_PKG_NAME").to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::INCREMENTAL,
                )),
                completion_provider: Some(CompletionOptions {
                    resolve_provider: Some(false),
                    trigger_characters: Some(vec![
                        "#".to_string(),
                        "^".to_string(),
                        "\"".to_string(),
                    ]),
                    work_done_progress_options: WorkDoneProgressOptions::default(),
                    all_commit_characters: None,
                    ..CompletionOptions::default()
                }),
                hover_provider: Some(tower_lsp::lsp_types::HoverProviderCapability::Simple(true)),
                definition_provider: Some(OneOf::Left(true)),
                references_provider: Some(OneOf::Right(ReferencesOptions {
                    work_done_progress_options: WorkDoneProgressOptions::default(),
                })),
                code_lens_provider: Some(CodeLensOptions {
                    resolve_provider: None,
                }),
                code_action_provider: Some(CodeActionProviderCapability::Options(
                    tower_lsp::lsp_types::CodeActionOptions {
                        code_action_kinds: Some(vec![CodeActionKind::QUICKFIX]),
                        resolve_provider: None,
                        work_done_progress_options: WorkDoneProgressOptions::default(),
                    },
                )),
                execute_command_provider: Some(ExecuteCommandOptions {
                    commands: vec!["turbo.run".to_string()],
                    work_done_progress_options: WorkDoneProgressOptions::default(),
                }),
                workspace: Some(WorkspaceServerCapabilities {
                    workspace_folders: Some(WorkspaceFoldersServerCapabilities {
                        supported: Some(true),
                        change_notifications: Some(OneOf::Left(true)),
                    }),
                    file_operations: None,
                }),
                ..ServerCapabilities::default()
            },
        })
    }

    async fn initialized(&self, _: InitializedParams) {}

    async fn shutdown(&self) -> tower_lsp::jsonrpc::Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        self.remember_open_file(&params);
        self.publish_diagnostics(params.text_document.uri, Some(params.text_document.version))
            .await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        self.remember_changed_file(&params);
        self.publish_diagnostics(
            params.text_document.uri.clone(),
            Some(params.text_document.version),
        )
        .await;
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        self.publish_diagnostics(params.text_document.uri, None)
            .await;
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        self.remember_closed_file(&params);
        self.client
            .publish_diagnostics(params.text_document.uri, Vec::new(), None)
            .await;
    }

    async fn completion(
        &self,
        _: CompletionParams,
    ) -> tower_lsp::jsonrpc::Result<Option<CompletionResponse>> {
        Ok(self.completions().await.map(CompletionResponse::Array))
    }

    async fn references(
        &self,
        params: ReferenceParams,
    ) -> tower_lsp::jsonrpc::Result<Option<Vec<Location>>> {
        Ok(self.references(&params).await)
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> tower_lsp::jsonrpc::Result<Option<GotoDefinitionResponse>> {
        Ok(self.goto_definition(&params).await)
    }

    async fn hover(&self, params: HoverParams) -> tower_lsp::jsonrpc::Result<Option<Hover>> {
        Ok(self.hover_markdown(&params).await)
    }

    async fn code_lens(
        &self,
        params: CodeLensParams,
    ) -> tower_lsp::jsonrpc::Result<Option<Vec<CodeLens>>> {
        Ok(self.code_lens(&params.text_document.uri))
    }

    async fn code_action(
        &self,
        params: CodeActionParams,
    ) -> tower_lsp::jsonrpc::Result<Option<CodeActionResponse>> {
        Ok(Some(self.quickfixes(&params)))
    }

    async fn execute_command(
        &self,
        _: ExecuteCommandParams,
    ) -> tower_lsp::jsonrpc::Result<Option<serde_json::Value>> {
        Ok(None)
    }

    async fn did_change_workspace_folders(&self, _: DidChangeWorkspaceFoldersParams) {}

    async fn did_change_configuration(&self, _: DidChangeConfigurationParams) {}

    async fn did_change_watched_files(&self, _: DidChangeWatchedFilesParams) {}
}

fn apply_content_changes(text: &mut String, changes: &[TextDocumentContentChangeEvent]) {
    for change in changes {
        match change.range {
            Some(range) => {
                if let Some(byte_range) = lsp_range_to_byte_range(text, range) {
                    text.replace_range(byte_range, &change.text);
                } else {
                    text.clone_from(&change.text);
                }
            }
            None => text.clone_from(&change.text),
        }
    }
}

fn lsp_range_to_byte_range(text: &str, range: Range) -> Option<std::ops::Range<usize>> {
    let start = utf16_position_to_byte_offset(text, range.start)?;
    let end = utf16_position_to_byte_offset(text, range.end)?;
    Some(start..end)
}

fn utf16_position_to_byte_offset(text: &str, position: Position) -> Option<usize> {
    let target_line = usize::try_from(position.line).ok()?;
    let target_character = usize::try_from(position.character).ok()?;

    let mut current_line = 0_usize;
    let mut line_start = 0_usize;

    for (byte_index, ch) in text.char_indices() {
        if current_line == target_line {
            break;
        }
        if ch == '\n' {
            current_line += 1;
            line_start = byte_index + ch.len_utf8();
        }
    }

    if current_line != target_line {
        return None;
    }

    let line_end = text[line_start..]
        .find('\n')
        .map_or(text.len(), |offset| line_start + offset);
    let line_text = &text[line_start..line_end];

    let mut utf16_units = 0_usize;
    for (byte_index, ch) in line_text.char_indices() {
        if utf16_units >= target_character {
            return Some(line_start + byte_index);
        }
        utf16_units += ch.len_utf16();
        if utf16_units > target_character {
            return Some(line_start + byte_index);
        }
    }

    if utf16_units == target_character {
        Some(line_end)
    } else {
        None
    }
}

fn hover_target_for_offset(text: &str, offset: usize) -> Option<HoverTarget> {
    let parse = parse_to_ast(text, &CollectOptions::default(), &ParseOptions::default()).ok()?;
    let root = parse.value.as_ref()?.as_object()?;

    for property in &root.properties {
        if key_range(property.range, property.name.as_str().len()).contains(&offset) {
            return Some(HoverTarget::TopLevelKey(property.name.as_str().to_string()));
        }

        if matches!(property.name.as_str(), "tasks" | "pipeline")
            && let Some(tasks) = property.value.as_object()
        {
            for task in &tasks.properties {
                if key_range(task.range, task.name.as_str().len()).contains(&offset) {
                    return Some(HoverTarget::TaskName(task.name.as_str().to_string()));
                }

                if let Some(task_object) = task.value.as_object() {
                    for field in &task_object.properties {
                        if key_range(field.range, field.name.as_str().len()).contains(&offset) {
                            return Some(HoverTarget::TaskField {
                                task_name: task.name.as_str().to_string(),
                                field_name: field.name.as_str().to_string(),
                            });
                        }

                        if field.name.as_str() == "dependsOn"
                            && let Some(array) = field.value.as_array()
                        {
                            for element in &array.elements {
                                if let Some(string) = element.as_string_lit()
                                    && collapse_string_range(string.range).contains(&offset)
                                {
                                    return Some(HoverTarget::DependsOnEntry {
                                        task_name: task.name.as_str().to_string(),
                                        entry: string.value.to_string(),
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    None
}

const fn key_range(range: JsonRange, key_len: usize) -> std::ops::Range<usize> {
    let start = range.start + 1;
    let end = start + key_len;
    start..end
}

const fn collapse_string_range(range: JsonRange) -> std::ops::Range<usize> {
    (range.start + 1)..range.end.saturating_sub(1)
}

fn byte_range_to_lsp_range(text: &str, range: std::ops::Range<usize>) -> Range {
    Range {
        start: byte_offset_to_position(text, range.start),
        end: byte_offset_to_position(text, range.end),
    }
}

fn byte_offset_to_position(text: &str, byte_offset: usize) -> Position {
    let clamped = byte_offset.min(text.len());
    let slice = &text[..clamped];
    let line = slice.bytes().filter(|byte| *byte == b'\n').count();
    let line_start = slice.rfind('\n').map_or(0, |index| index + 1);
    let character = text[line_start..clamped]
        .chars()
        .map(char::len_utf16)
        .sum::<usize>();

    Position {
        line: u32::try_from(line).unwrap_or(u32::MAX),
        character: u32::try_from(character).unwrap_or(u32::MAX),
    }
}

fn strip_lit_prefix<'a>(string: &'a StringLit<'a>, prefix: &str) -> Option<StringLit<'a>> {
    string.value.strip_prefix(prefix).map(|value| StringLit {
        value: value.into(),
        range: JsonRange {
            start: string.range.start + prefix.len(),
            end: string.range.end,
        },
    })
}

fn task_target_label(target: &HoverTarget) -> Option<String> {
    match target {
        HoverTarget::TaskName(name) => Some(name.clone()),
        HoverTarget::DependsOnEntry { entry, .. } => Some(
            entry
                .trim_start_matches('^')
                .trim_start_matches('$')
                .to_string(),
        ),
        HoverTarget::TaskField { .. } | HoverTarget::TopLevelKey(_) => None,
    }
}

fn task_definition_location(text: &str, uri: &Url, label: &str) -> Option<Location> {
    let parse = parse_to_ast(text, &CollectOptions::default(), &ParseOptions::default()).ok()?;
    let root = parse.value.as_ref()?.as_object()?;

    for task_group_name in ["tasks", "pipeline"] {
        let Some(task_group) = root.get_object(task_group_name) else {
            continue;
        };

        for property in &task_group.properties {
            if property.name.as_str() == label {
                let range = byte_range_to_lsp_range(text, key_range(property.range, label.len()));
                return Some(Location::new(uri.clone(), range));
            }
        }
    }

    None
}

fn report_invalid_packages_and_tasks(
    workspace: &WorkspaceState,
    text: &str,
    diagnostics: &mut Vec<Diagnostic>,
    package_task: &StringLit<'_>,
) {
    let task_ref = TaskReference::parse(package_task.value.as_ref());

    let range = byte_range_to_lsp_range(text, collapse_string_range(package_task.range));

    match (workspace.task_packages.get(task_ref.task), task_ref.package) {
        (_, Some(package_name)) if !workspace.package_names.contains(package_name) => {
            diagnostics.push(Diagnostic {
                message: format!("The package `{package_name}` does not exist in this workspace."),
                range,
                severity: Some(DiagnosticSeverity::ERROR),
                code: Some(NumberOrString::String("turbo:no-such-package".to_string())),
                ..Diagnostic::default()
            });
        }
        (Some(packages), Some(package_name))
            if !packages.iter().any(|name| name == package_name) =>
        {
            diagnostics.push(Diagnostic {
                message: format!(
                    "The task `{}` does not exist in package `{package_name}`.",
                    task_ref.task
                ),
                range,
                severity: Some(DiagnosticSeverity::ERROR),
                code: Some(NumberOrString::String(
                    "turbo:no-such-task-in-package".to_string(),
                )),
                ..Diagnostic::default()
            });
        }
        (None, _) => {
            diagnostics.push(Diagnostic {
                message: format!(
                    "The task `{}` does not exist in this workspace.",
                    task_ref.task
                ),
                range,
                severity: Some(DiagnosticSeverity::ERROR),
                code: Some(NumberOrString::String("turbo:no-such-task".to_string())),
                ..Diagnostic::default()
            });
        }
        _ => {}
    }
}

async fn load_root_package(root: &Path) -> Option<Package> {
    let package_json_path = root.join("package.json");
    let content = tokio::fs::read_to_string(&package_json_path).await.ok()?;
    let package_json: serde_json::Value = serde_json::from_str(&content).ok()?;
    let scripts = package_json
        .get("scripts")
        .and_then(|value| value.as_object())
        .map(|object| {
            object
                .iter()
                .filter_map(|(key, value)| {
                    value
                        .as_str()
                        .map(|script| (key.clone(), script.to_string()))
                })
                .collect()
        })
        .unwrap_or_default();

    Some(Package {
        name: package_json
            .get("name")
            .and_then(|value| value.as_str())
            .unwrap_or("root")
            .to_string(),
        path: root.to_path_buf(),
        package_json_path,
        scripts,
    })
}

fn script_location(content: &str, path: &Path, task_name: &str) -> Option<Location> {
    let needle = format!("\"{task_name}\"");
    let start = content.find(&needle)?;
    let end = start + needle.len();
    let range = byte_range_to_lsp_range(content, start..end);
    let uri = Url::from_file_path(path).ok()?;
    Some(Location::new(uri, range))
}

async fn build_hover_markdown(repo_root: Option<&Path>, target: &HoverTarget) -> String {
    let context = load_hover_context(repo_root).await;

    match target {
        HoverTarget::TopLevelKey(name) => top_level_hover(name),
        HoverTarget::TaskName(name) => task_name_hover(name, context.as_ref()),
        HoverTarget::TaskField {
            task_name,
            field_name,
        } => task_field_hover(task_name, field_name, context.as_ref()),
        HoverTarget::DependsOnEntry { task_name, entry } => {
            depends_on_hover(task_name, entry, context.as_ref())
        }
    }
}

#[derive(Debug, Clone)]
struct HoverContext {
    packages: Vec<Package>,
    root_path: PathBuf,
}

async fn load_hover_context(repo_root: Option<&Path>) -> Option<HoverContext> {
    let root = repo_root?;
    let discovery = PackageDiscovery::new(root.to_path_buf());
    let mut packages = discovery.discover_packages().await.ok()?;
    if let Some(root_package) = load_root_package(root).await
        && !packages
            .iter()
            .any(|package| package.path == root_package.path)
    {
        packages.push(root_package);
    }
    Some(HoverContext {
        packages,
        root_path: root.to_path_buf(),
    })
}

fn top_level_hover(name: &str) -> String {
    top_level_hover_meta(name).map_or_else(
        || {
            markdown_card(
                name,
                "Turbo config key.",
                "This key belongs to `turbo.json` / `turbo.jsonc`.",
                "{\n  \"tasks\": {}\n}",
                docs::CONFIGURATION,
            )
        },
        |meta| {
            let summary = meta
                .summary_override
                .or_else(|| top_level_summary(name))
                .unwrap_or("Turbo config key.");
            markdown_card(name, summary, meta.context, meta.example, meta.docs_url)
        },
    )
}

fn task_name_hover(name: &str, context: Option<&HoverContext>) -> String {
    let package_line = context
        .map(|ctx| packages_for_task(ctx, name))
        .filter(|packages| !packages.is_empty())
        .map_or_else(
            || "- Workspace script usage unknown until package discovery succeeds.".to_string(),
            |packages| {
                let joined = packages
                    .iter()
                    .take(4)
                    .copied()
                    .collect::<Vec<_>>()
                    .join(", ");
                let suffix = if packages.len() > 4 { ", ..." } else { "" };
                format!(
                    "- Implemented by {count} package(s): `{joined}`{suffix}",
                    count = packages.len(),
                )
            },
        );

    format!(
        "### `{name}`\n\nTurborepo task name.\n\n**Context**\n- Task key inside `tasks` / `pipeline`.\n{package_line}\n- Use `dependsOn` to wire execution order and cache boundaries.\n\n**Example**\n```jsonc\n{{\n  \"tasks\": {{\n    \"{name}\": {{\n      \"dependsOn\": [\"^build\"],\n      \"outputs\": [\"dist/**\"]\n    }}\n  }}\n}}\n```\n\n[Turbo task config docs]({})",
        docs::TASKS
    )
}

fn task_field_hover(task_name: &str, field_name: &str, context: Option<&HoverContext>) -> String {
    let card = task_field_hover_meta(field_name).map_or_else(
        || {
            markdown_card_with_context(
                field_name,
                "Turbo task field.",
                &format!("Current task: `{task_name}`."),
                "{\n  \"tasks\": {\n    \"build\": {}\n  }\n}",
                docs::TASKS,
            )
        },
        |meta| {
            let summary = meta
                .summary_override
                .or_else(|| task_field_summary(field_name))
                .unwrap_or("Turbo task field.");
            markdown_card_with_context(
                field_name,
                summary,
                &format!("Current task: `{task_name}`. {}", meta.context),
                meta.example,
                meta.docs_url,
            )
        },
    );

    card.replace(
        "Workspace script usage unknown until package discovery succeeds.",
        &context.map_or_else(
            || "Workspace script usage unknown until package discovery succeeds.".to_string(),
            |ctx| {
                let count = packages_for_task(ctx, task_name).len();
                format!("{count} package(s) currently expose `{task_name}` as a script.")
            },
        ),
    )
}

fn depends_on_hover(task_name: &str, entry: &str, context: Option<&HoverContext>) -> String {
    let meaning = entry.strip_prefix('^').map_or_else(
        || {
            let task_ref = TaskReference::parse(entry);
            if task_ref.package == Some(ROOT_PACKAGE_NAME) {
                format!("Targets root task `{}` in the workspace root.", task_ref.task)
            } else if let Some(package) = task_ref.package {
                format!("Targets task `{}` in package `{package}`.", task_ref.task)
            } else {
                format!("Targets task `{entry}` in the same package or root workspace.")
            }
        },
        |stripped| {
            format!("Runs `{stripped}` in dependency packages before `{task_name}` in the current package.")
        },
    );

    let package_hint = context
        .map(|ctx| packages_for_task(ctx, entry.trim_start_matches('^')))
        .filter(|packages| !packages.is_empty())
        .map(|packages| {
            format!(
                "- Seen in {} package(s): `{}`",
                packages.len(),
                packages
                    .iter()
                    .take(4)
                    .copied()
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
        .unwrap_or_default();

    format!(
        "### `{entry}`\n\nTask dependency reference.\n\n**Context**\n- Current task: `{task_name}`\n- {meaning}\n{package_hint}\n\n**Examples**\n```jsonc\n{{\n  \"tasks\": {{\n    \"{task_name}\": {{\n      \"dependsOn\": [\"^build\", \"lint\", \"web#codegen\"]\n    }}\n  }}\n}}\n```\n\n[Turbo `dependsOn` docs]({})",
        docs::DEPENDS_ON
    )
}

fn packages_for_task<'a>(context: &'a HoverContext, task_name: &str) -> Vec<&'a str> {
    let bare_task = TaskReference::parse(task_name.trim_start_matches('^')).task;

    context
        .packages
        .iter()
        .filter_map(|package| {
            if package.scripts.contains_key(bare_task) {
                if package.path == context.root_path {
                    Some(ROOT_PACKAGE_NAME)
                } else {
                    Some(package.name.as_str())
                }
            } else {
                None
            }
        })
        .collect()
}

fn markdown_card(
    title: &str,
    summary: &str,
    context: &str,
    example: &str,
    docs_url: &str,
) -> String {
    markdown_card_with_context(title, summary, context, example, docs_url)
}

fn markdown_card_with_context(
    title: &str,
    summary: &str,
    context: &str,
    example: &str,
    docs_url: &str,
) -> String {
    format!(
        "### `{title}`\n\n{summary}\n\n**Context**\n- {context}\n\n**Example**\n```jsonc\n{example}\n```\n\n[Turbo docs]({docs_url})"
    )
}

#[tokio::main]
async fn main() {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let (service, socket) = LspService::new(TurboBackend::new);
    Server::new(stdin, stdout, socket).serve(service).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_reference_parses_root_tasks() {
        let task_ref = TaskReference::parse("//#install:lsp");

        assert_eq!(task_ref.package, Some(ROOT_PACKAGE_NAME));
        assert_eq!(task_ref.task, "install:lsp");
    }

    #[test]
    fn utf16_position_handles_multibyte_chars() {
        let text = "a😀b\nsecond";
        let offset = utf16_position_to_byte_offset(
            text,
            Position {
                line: 0,
                character: 3,
            },
        );

        assert_eq!(offset, Some("a😀".len()));
    }

    #[test]
    fn hover_target_finds_depends_on_entries() {
        let text = r#"{
  "tasks": {
    "build": {
      "dependsOn": ["^build"]
    }
  }
}"#;

        let offset = text.find("^build").expect("dependsOn entry exists");
        let target = hover_target_for_offset(text, offset);

        assert_eq!(
            target,
            Some(HoverTarget::DependsOnEntry {
                task_name: "build".to_string(),
                entry: "^build".to_string(),
            })
        );
    }
}
