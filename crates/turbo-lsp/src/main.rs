use tokio::io;
use tower_lsp::{LspService, Server};
use turborepo_lsp::Backend;

#[tokio::main]
async fn main() {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let (service, socket) = LspService::new(Backend::new);
    Server::new(stdin, stdout, socket).serve(service).await;
}
