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
//! `initial` フィールドで指定されたファイルはハッシュなしでコピーする
//! （HTML から直接参照されるプレースホルダー画像などを想定）。
//! それ以外のファイル（index.html、HTML から直接参照される CSS/JS 等）は
//! ハッシュなしでそのままコピーする。
//!
//! ## manifest.json の strategies セクション
//! `src/assetsStrategy/` 配下のサブディレクトリを走査し、各ディレクトリの
//! `strategy.json` を読み込んで `manifest.json` の `strategies` セクションに追加する。
//! フォルダ名が `strategyAssets("name")` の呼び出し名と一致する。
//!
//! ## loader.js の出力
//! `output/loader.js` に `@statics-lead/loader` のブラウザバンドルを書き出す。
//! HTML から `<script type="module" src="/loader.js">` で読み込める。
//!
//! ## HTML 表示枠の自動注入（Issue #41）
//! `initial` が指定されたストラテジーについて、output/ 内の全 HTML ファイルを走査し、
//! `<div id="s3d-{name}">` または `<div id="{name}">` プレースホルダーを発見した場合、
//! その div 内に `<img src="/{initial_path}">` を自動注入する。
//!
//! ## loader.js への表示差し替えロジック注入（Issue #41）
//! `initial` と `files` 両方が指定されたストラテジーについて、
//! `output/loader.js` の末尾に自動初期化スクリプトを追記する。
//! このスクリプトは DOMContentLoaded 後に `strategyAssets()` を呼び出し、
//! CDN からロードされたアセットを初期画像と差し替える。

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

    // strategy.json の files に含まれるキーのみハッシュを付与する。
    // initial（プレースホルダー）はハッシュなし：HTML から直接参照されるため。
    let hashed_key_set: HashSet<String> = strategies
        .values()
        .flat_map(|s| s.files.iter().cloned())
        .collect();

    // ── 4. output/ へコピー
    // strategy.files に含まれるファイル → ハッシュ付きファイル名
    // strategy.initial で指定されたファイル → ハッシュなし（HTML 直参照のため）
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
        hashed_keys: hashed_key_set.clone(),
    };
    let mut manifest = build_manifest(&hashed, &manifest_opts).context("マニフェスト生成エラー")?;

    // ── 6. strategies セクションをセット
    manifest.strategies = strategies.clone();

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
    // HTML から `<script type="module" src="/loader.js">` で直接利用できる。
    let loader_path = output_dir.join("loader.js");
    {
        let mut loader_content = LOADER_BUNDLE.to_vec();
        // strategies に initial + files が指定されている場合、
        // 表示差し替えロジックを loader.js 末尾に自動追記する（Issue #41 Step 5）
        let autorun_js = generate_autorun_js(&strategies);
        if !autorun_js.is_empty() {
            loader_content.extend_from_slice(b"\n");
            loader_content.extend_from_slice(autorun_js.as_bytes());
        }
        std::fs::write(&loader_path, &loader_content)
            .with_context(|| format!("loader.js の書き込み失敗: {}", loader_path.display()))?;
    }

    // ── 9. output/ の HTML ファイルに initial 表示枠を自動注入（Issue #41 Step 4）
    inject_initial_into_html(&output_dir, &strategies)
        .context("HTML への initial 表示枠注入エラー")?;

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

    // initial: string（ファイルパス）または省略可
    // 旧スキーマ（bool）との後方互換: true → None、false → None として扱う
    let initial = match v.get("initial") {
        Some(serde_json::Value::String(s)) => Some(s.clone()),
        _ => None, // bool / null / 省略はすべて None
    };

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

/// `output/` 内の全 HTML ファイルを走査し、
/// `initial` が指定されたストラテジーに対して表示枠を自動注入する。
///
/// ## 注入ルール
/// - `<div id="s3d-{name}">` または `<div id="{name}">` プレースホルダーを検索
/// - 見つかった div が空（中身なし）の場合のみ注入する（既に内容がある場合はスキップ）
/// - 注入内容: `<img src="/{initial_path}" alt="{name}" class="s3d-initial" id="s3d-img-{name}" />`
/// - 見つからない場合は何もしない（HTML ファイルを強制変更しない）
pub fn inject_initial_into_html(
    output_dir: &Path,
    strategies: &HashMap<String, StrategyEntry>,
) -> Result<()> {
    // initial を持つストラテジーのみ対象
    let initial_strategies: Vec<(&String, &str)> = strategies
        .iter()
        .filter_map(|(name, entry)| entry.initial.as_deref().map(|p| (name, p)))
        .collect();

    if initial_strategies.is_empty() {
        return Ok(());
    }

    // output/ 内の *.html を再帰的に収集
    let html_files = collect_html_files(output_dir);

    for html_path in &html_files {
        let original = std::fs::read_to_string(html_path).with_context(|| {
            format!("HTML ファイルの読み込み失敗: {}", html_path.display())
        })?;
        let mut modified = original.clone();

        for (name, initial_path) in &initial_strategies {
            // <div id="s3d-{name}"> または <div id="{name}"> を検索
            let img_tag = format!(
                r#"<img src="/{initial_path}" alt="{name}" class="s3d-initial" id="s3d-img-{name}" />"#,
                initial_path = initial_path,
                name = name,
            );

            // s3d- プレフィックスあり優先、なしでもマッチ
            for div_id in &[format!("s3d-{}", name), name.to_string()] {
                // 空の div（中身なし）のみ注入: <div id="..."></div> または <div id="..." >  </div>
                let patterns = [
                    format!(r#"<div id="{div_id}"></div>"#),
                    format!(r#"<div id="{div_id}" ></div>"#),
                    format!(r#"<div id="{div_id}"> </div>"#),
                ];
                for pat in &patterns {
                    if modified.contains(pat.as_str()) {
                        let replacement = format!(
                            r#"<div id="{div_id}">{img_tag}</div>"#,
                            div_id = div_id,
                            img_tag = img_tag,
                        );
                        modified = modified.replacen(pat.as_str(), &replacement, 1);
                        break;
                    }
                }
            }
        }

        if modified != original {
            std::fs::write(html_path, &modified).with_context(|| {
                format!("HTML ファイルの書き込み失敗: {}", html_path.display())
            })?;
        }
    }

    Ok(())
}

/// `output_dir` 配下の全 `*.html` ファイルパスを再帰的に収集する。
fn collect_html_files(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut result = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                result.extend(collect_html_files(&path));
            } else if path.extension().and_then(|e| e.to_str()) == Some("html") {
                result.push(path);
            }
        }
    }
    result
}

/// `initial` と `files` 両方が指定されたストラテジーに対して、
/// DOMContentLoaded 後に `strategyAssets()`（ESM バンドル内部名 `V`）を呼び出し
/// CDN アセットを表示差し替える自動初期化スクリプトを生成する（Issue #41 Step 5）。
///
/// 生成される JS は `loader.js` の末尾に追記される。
/// ESM バンドル内の `V`（= `strategyAssets`）は同モジュールスコープで参照可能。
pub fn generate_autorun_js(
    strategies: &HashMap<String, StrategyEntry>,
) -> String {
    // initial かつ files 両方を持つストラテジーのみ対象
    let targets: Vec<(&String, &str, &Vec<String>)> = strategies
        .iter()
        .filter_map(|(name, entry)| {
            let initial = entry.initial.as_deref()?;
            if entry.files.is_empty() {
                return None;
            }
            Some((name, initial, &entry.files))
        })
        .collect();

    if targets.is_empty() {
        return String::new();
    }

    // 表示差し替えロジック
    // V は loader.bundle.js 内の strategyAssets の minified 名（同 ESM スコープ内で参照可能）
    let mut js = String::new();
    js.push_str("// s3d build 自動生成 — strategy 表示差し替えロジック（Issue #41）\n");
    js.push_str("(function(sa){\n");
    js.push_str("  'use strict';\n");
    js.push_str("  function s3dReplace(name){\n");
    js.push_str("    sa(name,{cache:true}).then(function(result){\n");
    js.push_str("      var keys=Object.keys(result.assets);\n");
    js.push_str("      if(keys.length===0)return;\n");
    js.push_str("      var first=result.assets[keys[0]];\n");
    js.push_str("      if(!first||!first.url)return;\n");
    js.push_str("      var img=document.getElementById('s3d-img-'+name);\n");
    js.push_str("      if(!img){var div=document.getElementById('s3d-'+name)||document.getElementById(name);if(div)img=div.querySelector('img');}\n");
    js.push_str("      if(img){img.src=first.url;img.classList.add('s3d-loaded');}\n");
    js.push_str("    }).catch(function(e){console.warn('[s3d] strategyAssets('+name+') failed',e);});\n");
    js.push_str("  }\n");
    js.push_str("  function s3dInit(){\n");

    for (name, _initial, _files) in &targets {
        js.push_str(&format!(
            "    s3dReplace({name_json});\n",
            name_json = serde_json::to_string(name.as_str()).unwrap_or_default(),
        ));
    }

    js.push_str("  }\n");
    js.push_str("  if(document.readyState==='loading'){\n");
    js.push_str("    document.addEventListener('DOMContentLoaded',s3dInit);\n");
    js.push_str("  }else{\n");
    js.push_str("    s3dInit();\n");
    js.push_str("  }\n");
    js.push_str("})(V);\n");

    js
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
        assert_eq!(sushi.initial, None); // initial: false → None（旧スキーマ後方互換）
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

    // ── Issue #41 新スキーマ テスト ─────────────────────────────────

    #[test]
    fn test_parse_strategy_json_initial_string() {
        // initial: "assets/placeholder.png" → Some("assets/placeholder.png")
        let json = r#"{
            "files": ["assets/hd.jpg"],
            "initial": "assets/placeholder.png",
            "cache": true
        }"#;
        let entry = parse_strategy_json(json, "test").unwrap();
        assert_eq!(entry.initial, Some("assets/placeholder.png".to_string()));
        assert_eq!(entry.files, vec!["assets/hd.jpg"]);
    }

    #[test]
    fn test_parse_strategy_json_initial_bool_backward_compat() {
        // 旧スキーマ initial: false → None（後方互換）
        let json = r#"{"files":["a.glb"],"initial":false,"cache":true}"#;
        let entry = parse_strategy_json(json, "test").unwrap();
        assert_eq!(entry.initial, None);

        // 旧スキーマ initial: true → None（後方互換）
        let json2 = r#"{"files":["a.glb"],"initial":true,"cache":true}"#;
        let entry2 = parse_strategy_json(json2, "test").unwrap();
        assert_eq!(entry2.initial, None);
    }

    #[test]
    fn test_parse_strategy_json_initial_omitted() {
        // initial 省略 → None
        let json = r#"{"files":["a.glb"],"cache":true}"#;
        let entry = parse_strategy_json(json, "test").unwrap();
        assert_eq!(entry.initial, None);
    }

    #[test]
    fn test_build_initial_file_has_no_hash_files_have_hash() {
        // Issue #41: initial で指定されたファイルはハッシュなし、
        // files で指定されたファイルはハッシュ付きでコピーされる
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("src");
        let assets_dir = src.join("assets");
        let strategy_dir = src.join("assetsStrategy").join("hero");
        std::fs::create_dir_all(&assets_dir).unwrap();
        std::fs::create_dir_all(&strategy_dir).unwrap();

        // initial: プレースホルダー画像（ハッシュなし）
        std::fs::write(assets_dir.join("placeholder.png"), b"small-png").unwrap();
        // files: CDN 配信する高解像度画像（ハッシュ付き）
        std::fs::write(assets_dir.join("hd.jpg"), b"large-jpg-data").unwrap();

        std::fs::write(
            strategy_dir.join("strategy.json"),
            r#"{
                "files": ["assets/hd.jpg"],
                "initial": "assets/placeholder.png",
                "cache": true
            }"#,
        ).unwrap();

        let config_path = dir.path().join("s3d.config.json");
        let cfg = make_config("src");
        crate::config::save_config(&config_path, &cfg).unwrap();

        run(&cfg, &config_path, false).unwrap();

        let output_assets: Vec<String> = std::fs::read_dir(dir.path().join("output/assets"))
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();

        // hd.jpg はハッシュ付き（files に指定）
        assert!(
            output_assets.iter().any(|f| f.starts_with("hd.") && f.ends_with(".jpg") && f != "hd.jpg"),
            "hd.jpg はハッシュ付きでコピーされるべき: {:?}", output_assets
        );
        // placeholder.png はハッシュなし（initial に指定）
        assert!(
            output_assets.contains(&"placeholder.png".to_string()),
            "placeholder.png はハッシュなしでコピーされるべき: {:?}", output_assets
        );
        assert!(
            !output_assets.iter().any(|f| f.starts_with("placeholder.") && f.ends_with(".png") && f != "placeholder.png"),
            "placeholder.png にハッシュが付いてはならない: {:?}", output_assets
        );

        // manifest の確認
        let content = std::fs::read_to_string(dir.path().join("output/manifest.json")).unwrap();
        let v: serde_json::Value = serde_json::from_str(&content).unwrap();

        // manifest.strategies.hero.initial が "assets/placeholder.png" になっている
        let initial = v["strategies"]["hero"]["initial"].as_str();
        assert_eq!(initial, Some("assets/placeholder.png"),
            "manifest の strategies.hero.initial が正しくない");

        // hd.jpg の URL はハッシュ付き
        let hd_url = v["assets"]["assets/hd.jpg"]["url"].as_str().unwrap();
        assert!(
            hd_url.starts_with('/') && hd_url.contains('.') && !hd_url.ends_with("/assets/hd.jpg"),
            "hd.jpg URL はルート相対 + ハッシュ付きであるべき: {hd_url}"
        );
        // placeholder.png の URL はハッシュなし
        let ph_url = v["assets"]["assets/placeholder.png"]["url"].as_str().unwrap();
        assert_eq!(ph_url, "/assets/placeholder.png",
            "placeholder.png URL はハッシュなしであるべき: {ph_url}");
    }

    // ── Issue #41 Step 4: HTML 表示枠注入テスト ─────────────────────────────

    #[test]
    fn test_inject_initial_into_html_basic() {
        // <div id="s3d-hero"> を注入して <img src="/assets/placeholder.png"> が入ること
        let dir = TempDir::new().unwrap();
        let output = dir.path().join("output");
        std::fs::create_dir_all(&output).unwrap();

        let html = r#"<!DOCTYPE html>
<html><body>
<div id="s3d-hero"></div>
</body></html>"#;
        std::fs::write(output.join("index.html"), html).unwrap();

        let mut strategies = HashMap::new();
        strategies.insert("hero".to_string(), StrategyEntry {
            files: vec!["assets/hd.jpg".to_string()],
            initial: Some("assets/placeholder.png".to_string()),
            cache: true,
            max_age: None,
            reload: None,
        });

        inject_initial_into_html(&output, &strategies).unwrap();

        let result = std::fs::read_to_string(output.join("index.html")).unwrap();
        assert!(
            result.contains(r#"<img src="/assets/placeholder.png""#),
            "initial img が注入されるべき: {result}"
        );
        assert!(
            result.contains(r#"id="s3d-img-hero""#),
            "img の id が s3d-img-hero であるべき: {result}"
        );
        assert!(
            result.contains(r#"class="s3d-initial""#),
            "img の class が s3d-initial であるべき: {result}"
        );
    }

    #[test]
    fn test_inject_initial_into_html_with_name_id() {
        // <div id="hero">（s3d- プレフィックスなし）でもマッチすること
        let dir = TempDir::new().unwrap();
        let output = dir.path().join("output");
        std::fs::create_dir_all(&output).unwrap();

        let html = r#"<!DOCTYPE html><html><body><div id="hero"></div></body></html>"#;
        std::fs::write(output.join("index.html"), html).unwrap();

        let mut strategies = HashMap::new();
        strategies.insert("hero".to_string(), StrategyEntry {
            files: vec!["assets/hd.jpg".to_string()],
            initial: Some("assets/ph.png".to_string()),
            cache: true,
            max_age: None,
            reload: None,
        });

        inject_initial_into_html(&output, &strategies).unwrap();

        let result = std::fs::read_to_string(output.join("index.html")).unwrap();
        assert!(
            result.contains(r#"src="/assets/ph.png""#),
            "initial img が注入されるべき（id なしプレフィックス）: {result}"
        );
    }

    #[test]
    fn test_inject_initial_skips_no_initial() {
        // initial が None のストラテジーは HTML を変更しない
        let dir = TempDir::new().unwrap();
        let output = dir.path().join("output");
        std::fs::create_dir_all(&output).unwrap();

        let html = r#"<!DOCTYPE html><html><body><div id="s3d-sushi"></div></body></html>"#;
        std::fs::write(output.join("index.html"), html.as_bytes()).unwrap();

        let mut strategies = HashMap::new();
        strategies.insert("sushi".to_string(), StrategyEntry {
            files: vec!["assets/sushi.glb".to_string()],
            initial: None, // initial なし
            cache: true,
            max_age: None,
            reload: None,
        });

        inject_initial_into_html(&output, &strategies).unwrap();

        let result = std::fs::read_to_string(output.join("index.html")).unwrap();
        assert_eq!(result, html, "initial なしの場合は HTML が変更されないべき");
    }

    // ── Issue #41 Step 5: loader.js autorun 生成テスト ───────────────────────

    #[test]
    fn test_generate_autorun_js_with_initial_and_files() {
        let mut strategies = HashMap::new();
        strategies.insert("hero".to_string(), StrategyEntry {
            files: vec!["assets/hd.jpg".to_string()],
            initial: Some("assets/placeholder.png".to_string()),
            cache: true,
            max_age: None,
            reload: None,
        });

        let js = generate_autorun_js(&strategies);
        assert!(!js.is_empty(), "initial + files があれば autorun JS が生成されるべき");
        assert!(js.contains("s3dInit"), "s3dInit 関数が含まれるべき: {js}");
        assert!(js.contains("\"hero\""), "ストラテジー名 hero が含まれるべき: {js}");
        assert!(js.contains("s3dReplace"), "s3dReplace 関数が含まれるべき: {js}");
        assert!(js.contains("DOMContentLoaded"), "DOMContentLoaded リスナーが含まれるべき: {js}");
        // V（strategyAssets の内部名）を呼び出していること
        assert!(js.contains(")(V)"), "V（strategyAssets）を IIFE 引数として渡すべき: {js}");
    }

    #[test]
    fn test_generate_autorun_js_no_initial_returns_empty() {
        // initial なしのストラテジーのみの場合は空文字列を返す
        let mut strategies = HashMap::new();
        strategies.insert("sushi".to_string(), StrategyEntry {
            files: vec!["assets/sushi.glb".to_string()],
            initial: None,
            cache: true,
            max_age: None,
            reload: None,
        });

        let js = generate_autorun_js(&strategies);
        assert!(js.is_empty(), "initial なしは空文字列を返すべき: {js}");
    }

    #[test]
    fn test_generate_autorun_js_empty_files_returns_empty() {
        // files が空の場合は autorun JS を生成しない
        let mut strategies = HashMap::new();
        strategies.insert("hero".to_string(), StrategyEntry {
            files: vec![], // files なし
            initial: Some("assets/placeholder.png".to_string()),
            cache: true,
            max_age: None,
            reload: None,
        });

        let js = generate_autorun_js(&strategies);
        assert!(js.is_empty(), "files 空は空文字列を返すべき: {js}");
    }

    #[test]
    fn test_build_loader_js_contains_autorun_when_initial_and_files() {
        // build を実行すると output/loader.js に autorun スクリプトが含まれる
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("src");
        let assets_dir = src.join("assets");
        let strategy_dir = src.join("assetsStrategy").join("hero");
        std::fs::create_dir_all(&assets_dir).unwrap();
        std::fs::create_dir_all(&strategy_dir).unwrap();

        std::fs::write(assets_dir.join("placeholder.png"), b"small").unwrap();
        std::fs::write(assets_dir.join("hd.jpg"), b"large").unwrap();

        std::fs::write(
            strategy_dir.join("strategy.json"),
            r#"{"files":["assets/hd.jpg"],"initial":"assets/placeholder.png","cache":true}"#,
        ).unwrap();

        let config_path = dir.path().join("s3d.config.json");
        let cfg = make_config("src");
        crate::config::save_config(&config_path, &cfg).unwrap();

        run(&cfg, &config_path, false).unwrap();

        let loader_content = std::fs::read_to_string(dir.path().join("output/loader.js")).unwrap();
        assert!(
            loader_content.contains("s3dInit"),
            "loader.js に autorun s3dInit が含まれるべき"
        );
        assert!(
            loader_content.contains("\"hero\""),
            "loader.js に hero ストラテジー名が含まれるべき"
        );
    }

    #[test]
    fn test_build_html_has_initial_img_injected() {
        // build を実行すると output/index.html に initial img が注入される
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("src");
        let assets_dir = src.join("assets");
        let strategy_dir = src.join("assetsStrategy").join("leaf");
        std::fs::create_dir_all(&assets_dir).unwrap();
        std::fs::create_dir_all(&strategy_dir).unwrap();

        std::fs::write(assets_dir.join("1KB.png"), b"png-data").unwrap();
        std::fs::write(assets_dir.join("leaf.jpg"), b"leaf-data").unwrap();

        // index.html にプレースホルダー div を記述
        let index_html = r#"<!DOCTYPE html>
<html lang="ja">
<head><meta charset="UTF-8"><title>Test</title></head>
<body>
<div id="s3d-leaf"></div>
<script type="module" src="/loader.js"></script>
</body>
</html>"#;
        std::fs::write(src.join("index.html"), index_html).unwrap();

        std::fs::write(
            strategy_dir.join("strategy.json"),
            r#"{"files":["assets/leaf.jpg"],"initial":"assets/1KB.png","cache":true}"#,
        ).unwrap();

        let config_path = dir.path().join("s3d.config.json");
        let cfg = make_config("src");
        crate::config::save_config(&config_path, &cfg).unwrap();

        run(&cfg, &config_path, false).unwrap();

        let html = std::fs::read_to_string(dir.path().join("output/index.html")).unwrap();
        assert!(
            html.contains(r#"src="/assets/1KB.png""#),
            "output/index.html に initial img が注入されるべき: {html}"
        );
        assert!(
            html.contains(r#"id="s3d-img-leaf""#),
            "img の id が s3d-img-leaf であるべき: {html}"
        );
    }
}
