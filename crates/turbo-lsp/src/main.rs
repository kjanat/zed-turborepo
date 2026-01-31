//! Turbo LSP Server
//!
//! Language Server Protocol implementation for Turborepo.

use tokio::io;
use tower_lsp::lsp_types::*;
use tower_lsp::{LspService, Server};
use turborepo_lsp::Backend;

/// Wrapper that adds server info to the upstream Backend
struct TurboBackend(Backend);

impl TurboBackend {
    fn new(client: tower_lsp::Client) -> Self {
        Self(Backend::new(client))
    }
}

#[tower_lsp::async_trait]
impl tower_lsp::LanguageServer for TurboBackend {
    async fn initialize(
        &self,
        params: InitializeParams,
    ) -> tower_lsp::jsonrpc::Result<InitializeResult> {
        let mut result = self.0.initialize(params).await?;
        // Add our server info
        result.server_info = Some(ServerInfo {
            name: env!("CARGO_PKG_NAME").to_string(),
            version: Some(env!("CARGO_PKG_VERSION").to_string()),
        });
        Ok(result)
    }

    async fn initialized(&self, params: InitializedParams) {
        self.0.initialized(params).await;
    }

    async fn shutdown(&self) -> tower_lsp::jsonrpc::Result<()> {
        self.0.shutdown().await
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        self.0.did_open(params).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        self.0.did_change(params).await;
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        self.0.did_save(params).await;
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        self.0.did_close(params).await;
    }

    async fn completion(
        &self,
        params: CompletionParams,
    ) -> tower_lsp::jsonrpc::Result<Option<CompletionResponse>> {
        self.0.completion(params).await
    }

    async fn references(
        &self,
        params: ReferenceParams,
    ) -> tower_lsp::jsonrpc::Result<Option<Vec<Location>>> {
        self.0.references(params).await
    }

    async fn code_lens(
        &self,
        params: CodeLensParams,
    ) -> tower_lsp::jsonrpc::Result<Option<Vec<CodeLens>>> {
        self.0.code_lens(params).await
    }

    async fn code_action(
        &self,
        params: CodeActionParams,
    ) -> tower_lsp::jsonrpc::Result<Option<CodeActionResponse>> {
        self.0.code_action(params).await
    }

    async fn execute_command(
        &self,
        params: ExecuteCommandParams,
    ) -> tower_lsp::jsonrpc::Result<Option<serde_json::Value>> {
        self.0.execute_command(params).await
    }

    async fn did_change_workspace_folders(&self, params: DidChangeWorkspaceFoldersParams) {
        self.0.did_change_workspace_folders(params).await;
    }

    async fn did_change_configuration(&self, params: DidChangeConfigurationParams) {
        self.0.did_change_configuration(params).await;
    }

    async fn did_change_watched_files(&self, params: DidChangeWatchedFilesParams) {
        self.0.did_change_watched_files(params).await;
    }
}

#[tokio::main]
async fn main() {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let (service, socket) = LspService::new(TurboBackend::new);
    Server::new(stdin, stdout, socket).serve(service).await;
}
