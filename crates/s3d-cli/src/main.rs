//! s3d CLI — エントリポイント

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use clap::{Parser, Subcommand};
use colored::Colorize;
use s3d_types::plugin::StoragePlugin;

mod commands;
pub mod config;
pub mod storage;

use config::{load_config, load_dotenv};

// ──────────────────────────────────────────────────────────────
// CLI 定義
// ──────────────────────────────────────────────────────────────

/// s3d — Static 3D Asset Deployment CLI
#[derive(Parser)]
#[command(name = "s3d", version, about, long_about = None)]
struct Cli {
    /// s3d.config.json のパス（デフォルト: ./s3d.config.json）
    #[arg(short, long, global = true, default_value = "s3d.config.json")]
    config: PathBuf,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// プロジェクトを初期化する
    Init,

    /// アセットをビルドしてマニフェストを生成する
    Build {
        /// ビルド前に output/ をクリーンしない（増分ビルド用）
        #[arg(long)]
        no_clean: bool,
    },

    /// 2 つのマニフェストの差分を表示する
    Diff {
        /// 旧 manifest.json のパス（省略時は初回デプロイとして扱う）
        #[arg(long)]
        old: Option<PathBuf>,
        /// 新 manifest.json のパス
        new: PathBuf,
    },

    /// アセットを R2/S3 へアップロードする
    Push {
        /// manifest.json のパスを上書き指定
        #[arg(long)]
        manifest: Option<PathBuf>,
        /// 実際の I/O を行わずプレビューのみ表示する
        #[arg(long)]
        dry_run: bool,
    },

    /// s3d.config.json と環境変数を検証する
    Validate,
}

// ──────────────────────────────────────────────────────────────
// main
// ──────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("{} {e:#}", "error:".red().bold());
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let cli = Cli::parse();

    // .env をロード（ファイルがない場合は無視）
    load_dotenv();

    match cli.command {
        Commands::Init => {
            commands::init::run()?;
        }

        Commands::Build { no_clean } => {
            let cfg = load_config(&cli.config)?;
            commands::build::run(&cfg, &cli.config, no_clean)?;
        }

        Commands::Diff { old, new } => {
            commands::diff::run(old.as_deref(), &new)?;
        }

        Commands::Push { manifest, dry_run } => {
            let cfg = load_config(&cli.config)?;
            let storage = build_storage(&cfg)?;
            commands::push::run(&cfg, &cli.config, manifest.as_deref(), dry_run, storage).await?;
        }

        Commands::Validate => {
            commands::validate::run(&cli.config)?;
        }
    }

    Ok(())
}

/// config から StoragePlugin を構築する
fn build_storage(cfg: &config::S3dCliConfig) -> Result<Arc<dyn StoragePlugin>> {
    use crate::config::CdnProvider;
    use crate::storage::{credentials::StorageCredentials, r2::R2Storage};

    let creds = StorageCredentials::from_env()?;
    let bucket = cfg.storage.bucket.clone();

    match cfg.storage.provider {
        CdnProvider::CloudflareR2 => {
            let account_id =
                cfg.storage.account_id.as_deref().ok_or_else(|| {
                    anyhow::anyhow!("storage.account_id が必要です (Cloudflare R2)")
                })?;
            let endpoint = cfg
                .storage
                .endpoint
                .clone()
                .unwrap_or_else(|| format!("https://{account_id}.r2.cloudflarestorage.com"));
            let storage = R2Storage::new(creds, bucket, endpoint, None)?;
            Ok(Arc::new(storage))
        }
        CdnProvider::AwsS3 | CdnProvider::Custom => {
            let endpoint = cfg
                .storage
                .endpoint
                .clone()
                .ok_or_else(|| anyhow::anyhow!("storage.endpoint が必要です (Custom/S3)"))?;
            let region = cfg.storage.region.clone();
            let storage = R2Storage::new(creds, bucket, endpoint, region)?;
            Ok(Arc::new(storage))
        }
    }
}
