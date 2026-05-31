use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use micropuffer::{Micropuffer, MiniStore};
use serde_json::Value;
use std::fs::{metadata, read_to_string, write};
use std::io::{Read, stdin};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(name = "micropuffer")]
#[command(about = "Local turbopuffer query emulator for dashboard mocks")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Query {
        #[arg(long)]
        store: PathBuf,
        #[arg(long)]
        namespace: String,
        #[arg(long)]
        request: Option<PathBuf>,
    },
    Write {
        #[arg(long)]
        store: PathBuf,
        #[arg(long)]
        namespace: String,
        #[arg(long)]
        request: Option<PathBuf>,
    },
    ListNamespaces {
        #[arg(long)]
        store: PathBuf,
        #[arg(long)]
        prefix: Option<String>,
        #[arg(long)]
        cursor: Option<String>,
        #[arg(long, default_value_t = 100)]
        page_size: usize,
    },
    DeleteNamespace {
        #[arg(long)]
        store: PathBuf,
        #[arg(long)]
        namespace: String,
    },
    Metadata {
        #[arg(long)]
        store: PathBuf,
        #[arg(long)]
        namespace: String,
    },
    Schema {
        #[arg(long)]
        store: PathBuf,
        #[arg(long)]
        namespace: String,
    },
    UpdateSchema {
        #[arg(long)]
        store: PathBuf,
        #[arg(long)]
        namespace: String,
        #[arg(long)]
        request: Option<PathBuf>,
    },
    PatchMetadata {
        #[arg(long)]
        store: PathBuf,
        #[arg(long)]
        namespace: String,
        #[arg(long)]
        request: Option<PathBuf>,
    },
    Export {
        #[arg(long)]
        store: PathBuf,
        #[arg(long)]
        namespace: String,
        #[arg(long)]
        request: Option<PathBuf>,
    },
    WarmCache {
        #[arg(long)]
        store: PathBuf,
        #[arg(long)]
        namespace: String,
    },
    Recall {
        #[arg(long)]
        store: PathBuf,
        #[arg(long)]
        namespace: String,
        #[arg(long)]
        request: Option<PathBuf>,
    },
    ExplainQuery {
        #[arg(long)]
        store: PathBuf,
        #[arg(long)]
        namespace: String,
        #[arg(long)]
        request: Option<PathBuf>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Query {
            store,
            namespace,
            request,
        } => run_query(store, &namespace, request),
        Command::Write {
            store,
            namespace,
            request,
        } => run_write(store, &namespace, request),
        Command::ListNamespaces {
            store,
            prefix,
            cursor,
            page_size,
        } => run_list_namespaces(store, prefix.as_deref(), cursor.as_deref(), page_size),
        Command::DeleteNamespace { store, namespace } => run_delete_namespace(store, &namespace),
        Command::Metadata { store, namespace } => run_metadata(store, &namespace),
        Command::Schema { store, namespace } => run_schema(store, &namespace),
        Command::UpdateSchema {
            store,
            namespace,
            request,
        } => run_update_schema(store, &namespace, request),
        Command::PatchMetadata {
            store,
            namespace,
            request,
        } => run_patch_metadata(store, &namespace, request),
        Command::Export {
            store,
            namespace,
            request,
        } => run_export(store, &namespace, request),
        Command::WarmCache { store, namespace } => run_warm_cache(store, &namespace),
        Command::Recall {
            store,
            namespace,
            request,
        } => run_recall(store, &namespace, request),
        Command::ExplainQuery {
            store,
            namespace,
            request,
        } => run_explain_query(store, &namespace, request),
    }
}

fn run_query(store_path: PathBuf, namespace: &str, request_path: Option<PathBuf>) -> Result<()> {
    let clone = Micropuffer::from_store(read_store(&store_path)?);
    let request = read_request(request_path)?;
    let response = clone.query(namespace, &request)?;
    print_json(&response)
}

fn run_write(store_path: PathBuf, namespace: &str, request_path: Option<PathBuf>) -> Result<()> {
    let mut clone = Micropuffer::from_store(read_store_or_default(&store_path)?);
    let request = read_request(request_path)?;
    let response = clone.write(namespace, &request)?;
    write_store(&store_path, clone.store())?;
    print_json(&response)
}

fn run_list_namespaces(
    store_path: PathBuf,
    prefix: Option<&str>,
    cursor: Option<&str>,
    page_size: usize,
) -> Result<()> {
    let clone = Micropuffer::from_store(read_store(&store_path)?);
    let response = clone.list_namespaces(prefix, cursor, page_size)?;
    print_json(&response)
}

fn run_delete_namespace(store_path: PathBuf, namespace: &str) -> Result<()> {
    let mut clone = Micropuffer::from_store(read_store_or_default(&store_path)?);
    let response = clone.delete_namespace(namespace)?;
    write_store(&store_path, clone.store())?;
    print_json(&response)
}

fn run_metadata(store_path: PathBuf, namespace: &str) -> Result<()> {
    let clone = Micropuffer::from_store(read_store(&store_path)?);
    let response = clone.metadata(namespace)?;
    print_json(&response)
}

fn run_schema(store_path: PathBuf, namespace: &str) -> Result<()> {
    let clone = Micropuffer::from_store(read_store(&store_path)?);
    let response = clone.schema(namespace)?;
    print_json(&response)
}

fn run_update_schema(
    store_path: PathBuf,
    namespace: &str,
    request_path: Option<PathBuf>,
) -> Result<()> {
    let mut clone = Micropuffer::from_store(read_store(&store_path)?);
    let request = read_request(request_path)?;
    let response = clone.update_schema(namespace, &request)?;
    write_store(&store_path, clone.store())?;
    print_json(&response)
}

fn run_patch_metadata(
    store_path: PathBuf,
    namespace: &str,
    request_path: Option<PathBuf>,
) -> Result<()> {
    let mut clone = Micropuffer::from_store(read_store(&store_path)?);
    let request = read_request(request_path)?;
    let response = clone.patch_metadata(namespace, &request)?;
    write_store(&store_path, clone.store())?;
    print_json(&response)
}

fn run_export(store_path: PathBuf, namespace: &str, request_path: Option<PathBuf>) -> Result<()> {
    let clone = Micropuffer::from_store(read_store(&store_path)?);
    let request = request_path
        .map(|path| read_request(Some(path)))
        .transpose()?
        .unwrap_or_else(|| serde_json::json!({}));
    let response = clone.export_namespace(namespace, &request)?;
    print_json(&response)
}

fn run_warm_cache(store_path: PathBuf, namespace: &str) -> Result<()> {
    let clone = Micropuffer::from_store(read_store(&store_path)?);
    let response = clone.warm_cache(namespace)?;
    print_json(&response)
}

fn run_recall(store_path: PathBuf, namespace: &str, request_path: Option<PathBuf>) -> Result<()> {
    let clone = Micropuffer::from_store(read_store(&store_path)?);
    let request = request_path
        .map(|path| read_request(Some(path)))
        .transpose()?
        .unwrap_or_else(|| serde_json::json!({}));
    let response = clone.recall(namespace, &request)?;
    print_json(&response)
}

fn run_explain_query(
    store_path: PathBuf,
    namespace: &str,
    request_path: Option<PathBuf>,
) -> Result<()> {
    let clone = Micropuffer::from_store(read_store(&store_path)?);
    let request = read_request(request_path)?;
    let response = clone.explain_query(namespace, &request)?;
    print_json(&response)
}

fn read_store(store_path: &PathBuf) -> Result<MiniStore> {
    let store_raw = read_to_string(store_path)
        .with_context(|| format!("failed to read store fixture {}", store_path.display()))?;
    serde_json::from_str(&store_raw)
        .with_context(|| format!("failed to parse store fixture {}", store_path.display()))
}

fn read_store_or_default(store_path: &PathBuf) -> Result<MiniStore> {
    if metadata(store_path)
        .map(|metadata| metadata.len() == 0)
        .unwrap_or(false)
    {
        return Ok(MiniStore::default());
    }
    match read_store(store_path) {
        Ok(store) => Ok(store),
        Err(error) if !store_path.exists() => Ok(MiniStore::default()),
        Err(error) => Err(error),
    }
}

fn write_store(store_path: &PathBuf, store: &MiniStore) -> Result<()> {
    let raw = serde_json::to_string_pretty(store)?;
    write(store_path, raw)
        .with_context(|| format!("failed to write store {}", store_path.display()))
}

fn read_request(request_path: Option<PathBuf>) -> Result<Value> {
    let request_raw = match request_path {
        Some(path) => read_to_string(&path)
            .with_context(|| format!("failed to read request {}", path.display()))?,
        None => {
            let mut input = String::new();
            stdin()
                .read_to_string(&mut input)
                .context("failed to read request from stdin")?;
            input
        }
    };
    serde_json::from_str(&request_raw).context("failed to parse request JSON")
}

fn print_json(response: &Value) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(&response)?);
    Ok(())
}
