//! `s3d build` — アセット収集・ハッシュ化・マニフェスト生成コマンド
//!
//! `src/` を読み取り、ファイルを `output/` にコピーして
//! `output/manifest.json` を生成する。
//!
//! ## クリーンビルド（デフォルト）
//! ビルド前に `output/` 内の全ファイルを削除する。
//! `--no-clean` オプションで削除をスキップできる（増分ビルド用）。
//!
//! ## ハッシュ付与ロジック
//! `src/assetsStrategy/**/strategy.json` の `files` フィールドに列挙されたファイルのみ
//! ハッシュ付きファイル名でコピーする（CDN 長期キャッシュ対象）。
//! それ以外のファイル（index.html、HTML から直接参照される CSS/JS/画像など）は
//! ハッシュなしでそのままコピーする。
//!
//! ## manifest.json の strategies セクション
//! `src/assetsStrategy/` 配下のサブディレクトリを走査し、各ディレクトリの
//! `strategy.json` を読み込んで `manifest.json` の `strategies` セクションに追加する。
//! フォルダ名が `strategyAssets("name")` の呼び出し名と一致する。
//!
//! ## loader.js の出力
//! `output/loader.js` に `@statics-lead/loader` のブラウザバンドルを書き出す。
//! HTML から `<script src="/loader.js">` で読み込める。

/// ビルド時に埋め込む loader.js ブラウザバンドル（IIFE）
const LOADER_BUNDLE: &[u8] = include_bytes!("../../../../packages/loader/loader.bundle.js");

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result};
use colored::Colorize;
use s3d_deploy::{
    collect::{collect, CollectOptions},
    hash::hash_assets,
    manifest::{build_manifest, manifest_to_json, ManifestOptions},
};
use s3d_types::manifest::{StrategyEntry, StrategyReload};

use crate::config::S3dCliConfig;

/// `s3d build` を実行する
///
/// `no_clean = false`（デフォルト）の場合、ビルド前に `output/` 内を全削除する。
/// `no_clean = true` の場合は削除せず増分コピーのみ行う。
pub fn run(config: &S3dCliConfig, config_path: &Path, no_clean: bool) -> Result<()> {
    let start = Instant::now();
    let project_root = config_path.parent().unwrap_or(Path::new("."));
    let src_dir = project_root.join(&config.src_dir);
    let output_dir = project_root.join(&config.output_dir);

    println!("{}", "s3d build — アセットをビルドします".bold().cyan());
    println!("  ソースディレクトリ : {}", src_dir.display());
    println!("  出力ディレクトリ   : {}", output_dir.display());

    if !src_dir.exists() {
        anyhow::bail!(
            "ソースディレクトリが見つかりません: {}\n`s3d init` を実行して src/ を生成してください。",
            src_dir.display()
        );
    }

    // ── 0. output/ クリーン（--no-clean 指定時はスキップ）
    if !no_clean && output_dir.exists() {
        clean_output_dir(&output_dir)
            .with_context(|| format!("output/ のクリーンに失敗: {}", output_dir.display()))?;
        println!("  {} output/ をクリーンしました", "✔".green());
    } else if no_clean {
        println!("  {} --no-clean: output/ のクリーンをスキップ", "ℹ".cyan());
    }

    // ── 1. 収集
    let collect_opts = CollectOptions {
        ignore: config.exclude.clone(),
        include: config.include.clone(),
        max_file_size: config.max_file_size.clone(),
    };
    let collected = collect(&src_dir, &collect_opts)
        .with_context(|| format!("アセット収集エラー: {}", src_dir.display()))?;
    println!("  収集: {} ファイル", collected.len().to_string().bold());

    // ── 2. ハッシュ化（全ファイル対象、コピー時に選別）
    let hashed = hash_assets(&collected, s3d_deploy::hash::DEFAULT_HASH_LENGTH)
        .context("ハッシュ計算エラー")?;

    // ── 3. assetsStrategy の files を収集してハッシュ付与対象セットを構築
    let strategies_root = src_dir.join("assetsStrategy");
    let strategies = scan_strategies(&strategies_root)
        .context("assetsStrategy ディレクトリの走査エラー")?;
    if !strategies.is_empty() {
        println!("  strategies: {} 件", strategies.len().to_string().bold());
    }

    // strategy.json の files に含まれるキーのみハッシュを付与する
    let hashed_key_set: HashSet<String> = strategies
        .values()
        .flat_map(|s| s.files.iter().cloned())
        .collect();

    // ── 4. output/ へコピー
    // assetsStrategy の files に含まれるファイル → ハッシュ付きファイル名
    // それ以外 → 元のキーのまま
    std::fs::create_dir_all(&output_dir)?;
    for asset in &hashed {
        let dest_key = if hashed_key_set.contains(&asset.key) {
            asset.hashed_key.clone()
        } else {
            asset.key.clone()
        };
        let dest = output_dir.join(&dest_key);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(&asset.absolute_path, &dest).with_context(|| {
            format!(
                "ファイルコピー失敗: {} → {}",
                asset.absolute_path.display(),
                dest.display()
            )
        })?;
    }

    // ── 5. マニフェスト生成
    // ローカルビルド時は CDN URL を埋め込まずルート相対 URL を使用する。
    // （例: /assets/sushi.abcd1234.glb）
    // 実際の CDN URL は s3d push 時に rewrite_urls_to_cdn() で書き換えられる。
    let manifest_opts = ManifestOptions {
        cdn_base_url: String::new(), // 空 = ルート相対 URL
        version: "1.0.0".to_string(),
        build_time: Some(chrono::Utc::now().to_rfc3339()),
        hashed_keys: hashed_key_set,
    };
    let mut manifest = build_manifest(&hashed, &manifest_opts).context("マニフェスト生成エラー")?;

    // ── 6. strategies セクションをセット
    manifest.strategies = strategies;

    // ── 7. manifest.json の書き込み
    let manifest_path = config.resolved_manifest_path();
    let manifest_path = if manifest_path.is_absolute() {
        manifest_path
    } else {
        project_root.join(&manifest_path)
    };
    if let Some(parent) = manifest_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = manifest_to_json(&manifest).context("マニフェスト JSON 変換エラー")?;
    std::fs::write(&manifest_path, &json)
        .with_context(|| format!("manifest.json の書き込み失敗: {}", manifest_path.display()))?;

    // ── 8. loader.js の書き出し
    // バイナリに埋め込んだ @statics-lead/loader ブラウザバンドルを output/ に書き出す。
    // HTML から `<script src="/loader.js">` で直接利用できる。
    let loader_path = output_dir.join("loader.js");
    std::fs::write(&loader_path, LOADER_BUNDLE)
        .with_context(|| format!("loader.js の書き込み失敗: {}", loader_path.display()))?;

    // ── サマリ表示
    let total_size: u64 = hashed.iter().map(|a| a.size).sum();
    let elapsed = start.elapsed();
    println!();
    println!("{}", "ビルド完了 ✔".green().bold());
    println!("  ファイル数   : {}", hashed.len().to_string().bold());
    println!("  合計サイズ   : {}", format_bytes(total_size).bold());
    println!(
        "  manifest.json: {}",
        manifest_path.display().to_string().bold()
    );
    println!("  ビルド時間   : {:.2}s", elapsed.as_secs_f64());

    Ok(())
}

/// `src/assetsStrategy/` 配下のサブディレクトリを走査し、
/// 各サブディレクトリの `strategy.json` を読み込んで `StrategyEntry` マップを返す。
///
/// - ルートの `strategy.json`（サブディレクトリでないもの）はスキップ
/// - `strategy.json` が存在しないサブディレクトリもスキップ（警告なし）
pub fn scan_strategies(strategies_root: &Path) -> Result<HashMap<String, StrategyEntry>> {
    let mut map = HashMap::new();

    if !strategies_root.exists() {
        return Ok(map);
    }

    let entries = std::fs::read_dir(strategies_root)
        .with_context(|| format!("assetsStrategy の読み込み失敗: {}", strategies_root.display()))?;

    for entry in entries {
        let entry = entry?;
        let path = entry.path();

        // サブディレクトリのみ対象
        if !path.is_dir() {
            continue;
        }

        let strategy_file = path.join("strategy.json");
        if !strategy_file.exists() {
            continue;
        }

        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        if name.is_empty() {
            continue;
        }

        let content = std::fs::read_to_string(&strategy_file).with_context(|| {
            format!("strategy.json の読み込み失敗: {}", strategy_file.display())
        })?;

        let strategy = parse_strategy_json(&content, &name).with_context(|| {
            format!(
                "strategy.json のパース失敗 ({}): {}",
                name,
                strategy_file.display()
            )
        })?;

        map.insert(name, strategy);
    }

    Ok(map)
}

/// strategy.json の JSON 文字列を `StrategyEntry` にパースする。
fn parse_strategy_json(content: &str, name: &str) -> Result<StrategyEntry> {
    let v: serde_json::Value =
        serde_json::from_str(content).with_context(|| format!("{} の JSON パース失敗", name))?;

    let files: Vec<String> = v
        .get("files")
        .and_then(|f| f.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    let initial = v
        .get("initial")
        .and_then(|x| x.as_bool())
        .unwrap_or(false);

    let cache = v.get("cache").and_then(|x| x.as_bool()).unwrap_or(true);

    let max_age = v
        .get("maxAge")
        .and_then(|x| x.as_str())
        .map(|s| s.to_string());

    let reload = v.get("reload").and_then(|r| {
        let trigger = r.get("trigger")?.as_str()?.to_string();
        let strategy = r.get("strategy")?.as_str()?.to_string();
        Some(StrategyReload { trigger, strategy })
    });

    Ok(StrategyEntry {
        files,
        initial,
        cache,
        max_age,
        reload,
    })
}

/// `output_dir` 内の全エントリを削除する（ディレクトリ自体は残す）。
///
/// ディレクトリエントリはサブツリーごと削除し、ファイル/シンボリックリンクは個別削除。
pub fn clean_output_dir(output_dir: &Path) -> Result<()> {
    for entry in std::fs::read_dir(output_dir)
        .with_context(|| format!("output/ の読み込み失敗: {}", output_dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            std::fs::remove_dir_all(&path)
                .with_context(|| format!("ディレクトリ削除失敗: {}", path.display()))?;
        } else {
            std::fs::remove_file(&path)
                .with_context(|| format!("ファイル削除失敗: {}", path.display()))?;
        }
    }
    Ok(())
}

pub fn format_bytes(bytes: u64) -> String {
    if bytes >= 1_073_741_824 {
        format!("{:.2} GB", bytes as f64 / 1_073_741_824.0)
    } else if bytes >= 1_048_576 {
        format!("{:.2} MB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1024 {
        format!("{:.2} KB", bytes as f64 / 1024.0)
    } else {
        format!("{} B", bytes)
    }
}

// ──────────────────────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{CdnProvider, S3dCliConfig, StorageConfig};
    use tempfile::TempDir;

    fn make_config(src_dir: &str) -> S3dCliConfig {
        S3dCliConfig {
            project: "test".to_string(),
            storage: StorageConfig {
                provider: CdnProvider::CloudflareR2,
                bucket: "bucket".to_string(),
                cdn_base_url: "https://cdn.example.com".to_string(),
                account_id: None,
                endpoint: None,
                region: None,
            },
            src_dir: src_dir.to_string(),
            output_dir: "output".to_string(),
            include: vec![],
            exclude: vec![],
            max_file_size: None,
            manifest_path: None,
            plugins: vec![],
        }
    }

    #[test]
    fn test_build_creates_manifest_and_hashed_files() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("app.js"), b"console.log('hello');").unwrap();
        std::fs::write(src.join("style.css"), b"body { margin: 0; }").unwrap();

        let config_path = dir.path().join("s3d.config.json");
        let cfg = make_config("src");
        crate::config::save_config(&config_path, &cfg).unwrap();

        run(&cfg, &config_path, false).unwrap();

        // manifest.json が生成されている
        let manifest_path = dir.path().join("output/manifest.json");
        assert!(manifest_path.exists(), "manifest.json が生成されていない");

        // マニフェストの内容確認
        let content = std::fs::read_to_string(&manifest_path).unwrap();
        let v: serde_json::Value = serde_json::from_str(&content).unwrap();
        let assets = v["assets"].as_object().unwrap();
        // 元のキー（app.js / style.css）がマニフェストキーになっている
        assert!(
            assets.contains_key("app.js"),
            "app.js がマニフェストに含まれていない: {content}"
        );
        assert!(
            assets.contains_key("style.css"),
            "style.css が含まれていない"
        );

        // strategy files がないので app.js / style.css はハッシュなし
        let output_files: Vec<_> = std::fs::read_dir(dir.path().join("output"))
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert!(
            output_files.contains(&"app.js".to_string()),
            "ハッシュなし app.js が output/ にあるべき: {output_files:?}"
        );
        assert!(
            output_files.contains(&"style.css".to_string()),
            "ハッシュなし style.css が output/ にあるべき: {output_files:?}"
        );
    }

    #[test]
    fn test_build_src_not_found() {
        let dir = TempDir::new().unwrap();
        // src/ を作らない
        let config_path = dir.path().join("s3d.config.json");
        let cfg = make_config("src");
        crate::config::save_config(&config_path, &cfg).unwrap();
        assert!(run(&cfg, &config_path, false).is_err());
    }

    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(500), "500 B");
        assert_eq!(format_bytes(1024), "1.00 KB");
        assert_eq!(format_bytes(1_048_576), "1.00 MB");
    }

    #[test]
    fn test_scan_strategies_empty_dir() {
        let dir = TempDir::new().unwrap();
        // assetsStrategy ディレクトリなし → 空マップ
        let result = scan_strategies(dir.path()).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_scan_strategies_with_subdir() {
        let dir = TempDir::new().unwrap();
        let sushi_dir = dir.path().join("sushi");
        std::fs::create_dir_all(&sushi_dir).unwrap();
        std::fs::write(
            sushi_dir.join("strategy.json"),
            r#"{"files":["assets/sushi.glb"],"initial":false,"cache":true,"maxAge":"7d","reload":{"trigger":"manifest-change","strategy":"diff"}}"#,
        )
        .unwrap();

        let result = scan_strategies(dir.path()).unwrap();
        assert_eq!(result.len(), 1);
        let sushi = result.get("sushi").expect("sushi エントリがない");
        assert_eq!(sushi.files, vec!["assets/sushi.glb"]);
        assert!(!sushi.initial);
        assert!(sushi.cache);
        assert_eq!(sushi.max_age.as_deref(), Some("7d"));
        let reload = sushi.reload.as_ref().expect("reload がない");
        assert_eq!(reload.trigger, "manifest-change");
        assert_eq!(reload.strategy, "diff");
    }

    #[test]
    fn test_scan_strategies_skips_root_json() {
        let dir = TempDir::new().unwrap();
        // ルートの strategy.json (サブディレクトリでない) はスキップされる
        std::fs::write(dir.path().join("strategy.json"), r#"{"files":[],"initial":false,"cache":true}"#).unwrap();
        let result = scan_strategies(dir.path()).unwrap();
        assert!(result.is_empty(), "ルート strategy.json はスキップされるべき");
    }

    #[test]
    fn test_scan_strategies_subdir_without_strategy_json() {
        let dir = TempDir::new().unwrap();
        // strategy.json のないサブディレクトリはスキップ
        std::fs::create_dir_all(dir.path().join("empty_strategy")).unwrap();
        let result = scan_strategies(dir.path()).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_build_manifest_contains_strategies() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("src");
        let assets_dir = src.join("assets");
        let strategy_dir = src.join("assetsStrategy").join("sushi");
        std::fs::create_dir_all(&assets_dir).unwrap();
        std::fs::create_dir_all(&strategy_dir).unwrap();

        // ダミーアセット
        std::fs::write(assets_dir.join("sushi.glb"), b"glb-data").unwrap();

        // sushi strategy
        std::fs::write(
            strategy_dir.join("strategy.json"),
            r#"{"files":["assets/sushi.glb"],"initial":false,"cache":true,"maxAge":"7d","reload":{"trigger":"manifest-change","strategy":"diff"}}"#,
        )
        .unwrap();

        let config_path = dir.path().join("s3d.config.json");
        let cfg = make_config("src");
        crate::config::save_config(&config_path, &cfg).unwrap();

        run(&cfg, &config_path, false).unwrap();

        let manifest_path = dir.path().join("output/manifest.json");
        let content = std::fs::read_to_string(&manifest_path).unwrap();
        let v: serde_json::Value = serde_json::from_str(&content).unwrap();

        // strategies セクションが存在する
        let strategies = v.get("strategies").expect("strategies セクションがない");
        let sushi = strategies.get("sushi").expect("sushi エントリがない");
        let files = sushi["files"].as_array().expect("files が配列でない");
        assert!(!files.is_empty(), "files が空");
        assert_eq!(sushi["cache"].as_bool(), Some(true));
        assert_eq!(sushi["maxAge"].as_str(), Some("7d"));
    }

    #[test]
    fn test_build_strategy_files_get_hash_others_dont() {
        // assetsStrategy files に含まれるファイル → ハッシュあり
        // それ以外 → ハッシュなし
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("src");
        let assets_dir = src.join("assets");
        let strategy_dir = src.join("assetsStrategy").join("sushi");
        std::fs::create_dir_all(&assets_dir).unwrap();
        std::fs::create_dir_all(&strategy_dir).unwrap();

        // strategy files に含まれる GLB
        std::fs::write(assets_dir.join("sushi.glb"), b"glb-data").unwrap();
        // strategy files に含まれない通常ファイル
        std::fs::write(src.join("index.html"), b"<!DOCTYPE html>").unwrap();
        std::fs::write(assets_dir.join("style.css"), b"body{}").unwrap();

        std::fs::write(
            strategy_dir.join("strategy.json"),
            r#"{"files":["assets/sushi.glb"],"initial":false,"cache":true}"#,
        )
        .unwrap();

        let config_path = dir.path().join("s3d.config.json");
        let cfg = make_config("src");
        crate::config::save_config(&config_path, &cfg).unwrap();

        run(&cfg, &config_path, false).unwrap();

        // output/assets/ 以下を収集
        let output_assets: Vec<_> = std::fs::read_dir(dir.path().join("output/assets"))
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        let output_root: Vec<_> = std::fs::read_dir(dir.path().join("output"))
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();

        // sushi.glb はハッシュ付き
        assert!(
            output_assets.iter().any(|f| f.starts_with("sushi.") && f.ends_with(".glb") && f != "sushi.glb"),
            "sushi.glb にハッシュが付くべき: {:?}", output_assets
        );
        // style.css はハッシュなし
        assert!(
            output_assets.contains(&"style.css".to_string()),
            "style.css はハッシュなしのまま: {:?}", output_assets
        );
        // index.html はハッシュなし
        assert!(
            output_root.contains(&"index.html".to_string()),
            "index.html はハッシュなしのまま: {:?}", output_root
        );

        // manifest の URL 確認
        let content = std::fs::read_to_string(dir.path().join("output/manifest.json")).unwrap();
        let v: serde_json::Value = serde_json::from_str(&content).unwrap();
        let assets = v["assets"].as_object().unwrap();

        // ビルド時は相対 URL（CDN ベースなし）
        // sushi.glb の URL にはハッシュが入り、かつルート相対 URL
        let sushi_url = assets["assets/sushi.glb"]["url"].as_str().unwrap();
        assert!(
            sushi_url.starts_with('/') && sushi_url.contains('.') && !sushi_url.ends_with("/assets/sushi.glb"),
            "sushi.glb URL はルート相対 + ハッシュ付きであるべき: {sushi_url}"
        );
        // style.css の URL はハッシュなし（ルート相対）
        let css_url = assets["assets/style.css"]["url"].as_str().unwrap();
        assert!(
            css_url.starts_with('/') && css_url.ends_with("/assets/style.css"),
            "style.css URL はルート相対 + ハッシュなしであるべき: {css_url}"
        );
        // index.html の URL はハッシュなし（ルート相対）
        let html_url = assets["index.html"]["url"].as_str().unwrap();
        assert!(
            html_url.starts_with('/') && html_url.ends_with("/index.html"),
            "index.html URL はルート相対 + ハッシュなしであるべき: {html_url}"
        );
    }

    #[test]
    fn test_build_excludes_gitkeep_and_assets_strategy() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("src");
        let assets_dir = src.join("assets");
        let strategy_dir = src.join("assetsStrategy").join("sushi");
        std::fs::create_dir_all(&assets_dir).unwrap();
        std::fs::create_dir_all(&strategy_dir).unwrap();

        // .gitkeep と assetsStrategy/ 配下は collect から除外される
        std::fs::write(assets_dir.join(".gitkeep"), b"").unwrap();
        std::fs::write(strategy_dir.join("strategy.json"), b"{\"files\":[],\"initial\":false,\"cache\":true}").unwrap();
        std::fs::write(assets_dir.join("hero.png"), b"png-data").unwrap();

        let config_path = dir.path().join("s3d.config.json");
        let cfg = make_config("src");
        crate::config::save_config(&config_path, &cfg).unwrap();

        run(&cfg, &config_path, false).unwrap();

        let manifest_path = dir.path().join("output/manifest.json");
        let content = std::fs::read_to_string(&manifest_path).unwrap();
        let v: serde_json::Value = serde_json::from_str(&content).unwrap();
        let assets = v["assets"].as_object().unwrap();

        // .gitkeep はマニフェストに含まれない
        assert!(
            !assets.keys().any(|k| k.contains(".gitkeep")),
            ".gitkeep がマニフェストに含まれていてはならない: {:?}",
            assets.keys().collect::<Vec<_>>()
        );
        // assetsStrategy/ 配下もマニフェストの assets に含まれない
        assert!(
            !assets.keys().any(|k| k.starts_with("assetsStrategy")),
            "assetsStrategy/ がマニフェストの assets に含まれていてはならない: {:?}",
            assets.keys().collect::<Vec<_>>()
        );
        // hero.png は含まれる
        assert!(
            assets.keys().any(|k| k.contains("hero.png")),
            "hero.png がマニフェストに含まれるべき: {:?}",
            assets.keys().collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_build_outputs_loader_js() {
        // s3d build が output/loader.js を生成することを確認
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("index.html"), b"<!DOCTYPE html>").unwrap();

        let config_path = dir.path().join("s3d.config.json");
        let cfg = make_config("src");
        crate::config::save_config(&config_path, &cfg).unwrap();

        run(&cfg, &config_path, false).unwrap();

        let loader_path = dir.path().join("output/loader.js");
        assert!(loader_path.exists(), "output/loader.js が生成されていない");

        // loader.js の内容が IIFE バンドルであることを確認
        let content = std::fs::read(&loader_path).unwrap();
        assert!(!content.is_empty(), "output/loader.js が空");
    }

    #[test]
    fn test_build_manifest_urls_are_relative() {
        // build 時はマニフェストの URL がルート相対 URL（CDN URL なし）になる
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("app.js"), b"console.log(1);").unwrap();
        std::fs::write(src.join("index.html"), b"<!DOCTYPE html>").unwrap();

        let config_path = dir.path().join("s3d.config.json");
        let cfg = make_config("src");
        crate::config::save_config(&config_path, &cfg).unwrap();

        run(&cfg, &config_path, false).unwrap();

        let content = std::fs::read_to_string(dir.path().join("output/manifest.json")).unwrap();
        let v: serde_json::Value = serde_json::from_str(&content).unwrap();
        let assets = v["assets"].as_object().unwrap();

        // 全 URL がルート相対（"/" で始まる）で CDN URL（"https://"）を含まない
        for (key, asset) in assets {
            let url = asset["url"].as_str().unwrap();
            assert!(
                url.starts_with('/'),
                "ビルド時の URL はルート相対であるべき: key={key}, url={url}"
            );
            assert!(
                !url.starts_with("https://"),
                "ビルド時は CDN URL が含まれてはならない: key={key}, url={url}"
            );
        }
    }

    // ──────────────────────────────────────────────────────────
    // clean / no-clean テスト
    // ──────────────────────────────────────────────────────────

    #[test]
    fn test_build_cleans_output_by_default() {
        // デフォルト(no_clean=false)では前回のファイルが削除される
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("app.js"), b"console.log(1);").unwrap();

        let config_path = dir.path().join("s3d.config.json");
        let cfg = make_config("src");
        crate::config::save_config(&config_path, &cfg).unwrap();

        // 1回目のビルド
        run(&cfg, &config_path, false).unwrap();

        // output/ に古い残留ファイルを手動で配置
        let output_dir = dir.path().join("output");
        std::fs::write(output_dir.join("stale_file.txt"), b"old data").unwrap();
        assert!(output_dir.join("stale_file.txt").exists(), "前提: stale_file.txt が存在する");

        // 2回目のビルド（no_clean=false）→ 古いファイルが削除される
        run(&cfg, &config_path, false).unwrap();

        assert!(
            !output_dir.join("stale_file.txt").exists(),
            "stale_file.txt はクリーンで削除されるべき"
        );
        // manifest.json は再生成されている
        assert!(output_dir.join("manifest.json").exists());
    }

    #[test]
    fn test_build_no_clean_keeps_stale_files() {
        // --no-clean では前回のファイルが残る
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("app.js"), b"console.log(1);").unwrap();

        let config_path = dir.path().join("s3d.config.json");
        let cfg = make_config("src");
        crate::config::save_config(&config_path, &cfg).unwrap();

        // 1回目のビルド
        run(&cfg, &config_path, false).unwrap();

        // output/ に古い残留ファイルを手動で配置
        let output_dir = dir.path().join("output");
        std::fs::write(output_dir.join("stale_file.txt"), b"old data").unwrap();

        // 2回目のビルド（no_clean=true）→ 古いファイルが残る
        run(&cfg, &config_path, true).unwrap();

        assert!(
            output_dir.join("stale_file.txt").exists(),
            "--no-clean では stale_file.txt が残るべき"
        );
    }

    #[test]
    fn test_clean_output_dir_removes_files_and_subdirs() {
        let dir = TempDir::new().unwrap();
        let output = dir.path().join("output");
        std::fs::create_dir_all(&output).unwrap();

        // ファイルとサブディレクトリを配置
        std::fs::write(output.join("file.txt"), b"data").unwrap();
        std::fs::write(output.join("manifest.json"), b"{}").unwrap();
        let sub = output.join("assets");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("img.png"), b"img").unwrap();

        clean_output_dir(&output).unwrap();

        // output/ 自体は残る
        assert!(output.exists(), "output/ ディレクトリ自体は残るべき");
        // 中身は空
        let entries: Vec<_> = std::fs::read_dir(&output).unwrap().collect();
        assert!(entries.is_empty(), "output/ の中身が空になるべき: {:?}", entries);
    }
}
