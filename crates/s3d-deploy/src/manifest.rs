//! マニフェスト生成モジュール
//!
//! [`HashedAsset`] のリストと CDN base URL・バージョン文字列から
//! [`DeployManifest`] を構築し JSON にシリアライズする。
//!
//! MIME タイプは拡張子から推定する。
//! glTF アセット (`.gltf` / `.glb`) の依存関係（`.bin` / テクスチャ）は
//! ファイル内容を解析して自動的にマッピングする。

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use s3d_types::asset::HashedAsset;
use s3d_types::manifest::{AssetEntry, DeployManifest};
use thiserror::Error;

/// manifest モジュールのエラー型
#[derive(Debug, Error)]
pub enum ManifestError {
    #[error("I/O error reading `{path}`: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),
}

/// ファイル拡張子から MIME タイプを推定する。
///
/// `mime_guess` が `application/octet-stream` を返した場合も含め、
/// 3D アセット特有の拡張子を優先的に解決する。
pub fn guess_content_type(key: &str) -> String {
    // 3D/ゲームアセット特有の拡張子を先に処理
    let ext = key.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "glb" => return "model/gltf-binary".to_string(),
        "gltf" => return "model/gltf+json".to_string(),
        "bin" => return "application/octet-stream".to_string(),
        "ktx2" => return "image/ktx2".to_string(),
        "basis" => return "image/basis".to_string(),
        "draco" | "drc" => return "application/octet-stream".to_string(),
        _ => {}
    }
    mime_guess::from_path(key)
        .first_or_octet_stream()
        .to_string()
}

/// glTF JSON ファイルから参照される外部バッファ・テクスチャの URI リストを抽出する。
///
/// 解析に失敗した場合は空のベクタを返す（non-fatal）。
fn extract_gltf_dependencies(content: &[u8], hashed_assets: &[HashedAsset]) -> Vec<String> {
    // glTF は JSON なので serde_json でパース
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(content) else {
        return vec![];
    };

    let mut uris: Vec<String> = Vec::new();

    // buffers[].uri
    if let Some(buffers) = value.get("buffers").and_then(|v| v.as_array()) {
        for buf in buffers {
            if let Some(uri) = buf.get("uri").and_then(|v| v.as_str()) {
                uris.push(uri.to_string());
            }
        }
    }
    // images[].uri
    if let Some(images) = value.get("images").and_then(|v| v.as_array()) {
        for img in images {
            if let Some(uri) = img.get("uri").and_then(|v| v.as_str()) {
                uris.push(uri.to_string());
            }
        }
    }

    // URI をハッシュ付きキーに解決する
    uris.into_iter()
        .filter_map(|uri| {
            // data URI はスキップ
            if uri.starts_with("data:") {
                return None;
            }
            // hashed_assets の中で元のキーのファイル名部分が一致するものを探す
            hashed_assets
                .iter()
                .find(|a| {
                    let filename = a.key.rsplit('/').next().unwrap_or(&a.key);
                    filename == uri || a.key == uri
                })
                .map(|a| a.hashed_key.clone())
        })
        .collect()
}

/// マニフェスト構築オプション
#[derive(Debug, Clone)]
pub struct ManifestOptions {
    /// CDN の base URL（末尾スラッシュなし）。
    /// `build` 時は空文字を渡してルート相対 URL を生成し、
    /// `push` 時に CDN 絶対 URL へ書き換える。
    pub cdn_base_url: String,
    /// デプロイバージョン（例: `"1.0.0"`）
    pub version: String,
    /// ビルド日時（RFC3339）。`None` の場合は現在時刻の代わりに空文字を使う
    pub build_time: Option<String>,
    /// ハッシュを付与するファイルキーのセット（assetsStrategy の files に含まれるもの）
    /// 一致するファイルは hashed_key を URL に使用し、それ以外は元の key をそのまま使用する
    pub hashed_keys: HashSet<String>,
}

/// [`HashedAsset`] のリストから [`DeployManifest`] を構築する。
///
/// - `cdn_base_url` が空文字の場合はルート相対 URL（`/{key}`）を生成する
///   （`s3d build` 用 — CORS を避けるためローカルでは相対 URL を使用）
/// - `cdn_base_url` が指定された場合は `{cdn_base_url}/{key}` の絶対 URL を生成する
///   （`s3d push` が manifest を CDN 用に書き換える際に使用）
/// - content_type は拡張子から推定
/// - `.gltf` ファイルは依存関係を自動解析
pub fn build_manifest(
    assets: &[HashedAsset],
    opts: &ManifestOptions,
) -> Result<DeployManifest, ManifestError> {
    let base = opts.cdn_base_url.trim_end_matches('/');
    let build_time = opts
        .build_time
        .clone()
        .unwrap_or_else(|| String::from("1970-01-01T00:00:00Z"));

    let mut entries: HashMap<String, AssetEntry> = HashMap::new();

    for asset in assets {
        // hashed_keys に含まれるファイルのみハッシュ付き URL、それ以外は元のキーをそのまま使う
        let url_path = if opts.hashed_keys.contains(&asset.key) {
            asset.hashed_key.clone()
        } else {
            asset.key.clone()
        };
        // cdn_base_url が空ならルート相対 URL（例: /assets/sushi.abcd1234.glb）
        let url = if base.is_empty() {
            format!("/{}", url_path)
        } else {
            format!("{}/{}", base, url_path)
        };
        let content_type = guess_content_type(&asset.key);

        // glTF JSON の依存関係解析
        let dependencies = if asset.key.ends_with(".gltf") {
            let content =
                std::fs::read(&asset.absolute_path).map_err(|source| ManifestError::Io {
                    path: asset.absolute_path.clone(),
                    source,
                })?;
            let deps = extract_gltf_dependencies(&content, assets);
            if deps.is_empty() {
                None
            } else {
                Some(deps)
            }
        } else {
            None
        };

        entries.insert(
            asset.key.clone(), // 元のキーでインデックス（diff の比較が正しく機能する）
            AssetEntry {
                url,
                size: asset.size,
                hash: asset.hash.clone(),
                content_type,
                dependencies,
            },
        );
    }

    Ok(DeployManifest {
        schema_version: 1,
        version: opts.version.clone(),
        build_time,
        assets: entries,
        strategies: HashMap::new(),
    })
}

/// `build` 時に生成したルート相対 URL を CDN 絶対 URL に書き換える。
///
/// `s3d push` が呼び出す。
/// 相対 URL (`/` で始まる) のみ書き換え、すでに `http` または `https` で始まる
/// URL はそのままにする（冪等性のため）。
pub fn rewrite_urls_to_cdn(manifest: &mut DeployManifest, cdn_base_url: &str) {
    let base = cdn_base_url.trim_end_matches('/');
    for entry in manifest.assets.values_mut() {
        if entry.url.starts_with('/') {
            // 先頭の `/` を除いたパスを CDN base に結合する
            let path = entry.url.trim_start_matches('/');
            entry.url = format!("{}/{}", base, path);
        }
    }
}

/// [`DeployManifest`] を pretty-print JSON 文字列に変換する。
pub fn manifest_to_json(manifest: &DeployManifest) -> Result<String, ManifestError> {
    Ok(serde_json::to_string_pretty(manifest)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn make_hashed(key: &str, hash: &str, size: u64, path: PathBuf) -> HashedAsset {
        let hashed_key = crate::hash::insert_hash_into_key(key, hash);
        let hashed_filename = hashed_key
            .rsplit('/')
            .next()
            .unwrap_or(&hashed_key)
            .to_string();
        HashedAsset {
            key: key.to_string(),
            absolute_path: path,
            size,
            hash: hash.to_string(),
            hashed_filename,
            hashed_key,
        }
    }

    #[test]
    fn build_manifest_basic() {
        let tmp = tempfile::TempDir::new().unwrap();
        let js_path = tmp.path().join("main.js");
        std::fs::write(&js_path, b"console.log(1)").unwrap();

        let assets = vec![make_hashed("js/main.js", "abcd1234", 14, js_path)];

        let mut hashed_keys = HashSet::new();
        hashed_keys.insert("js/main.js".to_string());
        let opts = ManifestOptions {
            cdn_base_url: String::new(), // build 時は空（ルート相対 URL）
            version: "1.0.0".to_string(),
            build_time: Some("2026-03-20T00:00:00Z".to_string()),
            hashed_keys,
        };
        let manifest = build_manifest(&assets, &opts).unwrap();
        assert_eq!(manifest.version, "1.0.0");
        // キーは元のパス（ハッシュなし）
        assert!(manifest.assets.contains_key("js/main.js"));

        let entry = &manifest.assets["js/main.js"];
        // hashed_keys に含まれるので hashed_key ベースのルート相対 URL
        assert_eq!(entry.url, "/js/main.abcd1234.js");
        // mime_guess は "text/javascript" または "application/javascript" を返す
        assert!(entry.content_type.contains("javascript"));
        assert_eq!(entry.size, 14);
        assert_eq!(entry.hash, "abcd1234");
        assert!(entry.dependencies.is_none());
    }

    #[test]
    fn guess_content_type_3d_assets() {
        assert_eq!(guess_content_type("model.glb"), "model/gltf-binary");
        assert_eq!(guess_content_type("scene.gltf"), "model/gltf+json");
        assert_eq!(guess_content_type("texture.ktx2"), "image/ktx2");
        assert_eq!(guess_content_type("buffer.bin"), "application/octet-stream");
    }

    #[test]
    fn guess_content_type_web_assets() {
        assert!(guess_content_type("app.js").contains("javascript"));
        assert!(guess_content_type("style.css").contains("css"));
        assert!(guess_content_type("image.png").contains("png"));
    }

    #[test]
    fn manifest_to_json_is_valid() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("a.js");
        std::fs::write(&path, b"x").unwrap();

        let assets = vec![make_hashed("a.js", "11223344", 1, path)];
        let opts = ManifestOptions {
            cdn_base_url: "https://cdn.test".to_string(),
            version: "0.1.0".to_string(),
            build_time: Some("2026-01-01T00:00:00Z".to_string()),
            hashed_keys: HashSet::new(),
        };
        let manifest = build_manifest(&assets, &opts).unwrap();
        let json = manifest_to_json(&manifest).unwrap();
        // JSON として再パース可能
        let reparsed: DeployManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(reparsed.version, "0.1.0");
    }

    #[test]
    fn gltf_dependencies_extracted() {
        let tmp = tempfile::TempDir::new().unwrap();

        // buffer.bin を用意
        let bin_path = tmp.path().join("buffer.bin");
        std::fs::write(&bin_path, &[0u8; 16]).unwrap();

        // scene.gltf を用意（buffer.bin を参照）
        let gltf_content = serde_json::json!({
            "asset": { "version": "2.0" },
            "buffers": [{ "uri": "buffer.bin", "byteLength": 16 }],
            "images": []
        })
        .to_string();
        let gltf_path = tmp.path().join("scene.gltf");
        std::fs::write(&gltf_path, gltf_content.as_bytes()).unwrap();

        let assets = vec![
            make_hashed(
                "scene.gltf",
                "aaaa0001",
                gltf_content.len() as u64,
                gltf_path,
            ),
            make_hashed("buffer.bin", "bbbb0002", 16, bin_path),
        ];

        let mut hashed_keys = HashSet::new();
        hashed_keys.insert("scene.gltf".to_string());
        hashed_keys.insert("buffer.bin".to_string());
        let opts = ManifestOptions {
            cdn_base_url: "https://cdn.example.com".to_string(),
            version: "1.0.0".to_string(),
            build_time: None,
            hashed_keys,
        };

        let manifest = build_manifest(&assets, &opts).unwrap();
        // gltf エントリのキーは元のパス（ハッシュなし）
        let gltf_entry = manifest.assets.get("scene.gltf").unwrap();
        let deps = gltf_entry.dependencies.as_ref().unwrap();
        // 依存先の URL は hashed_key ベース
        assert!(deps.contains(&"buffer.bbbb0002.bin".to_string()));
    }

    #[test]
    fn build_manifest_with_cdn_base_url() {
        // cdn_base_url 指定時は絶対 URL が生成される（push シナリオ）
        let tmp = tempfile::TempDir::new().unwrap();
        let js_path = tmp.path().join("main.js");
        std::fs::write(&js_path, b"x").unwrap();

        let assets = vec![make_hashed("js/main.js", "abcd1234", 1, js_path)];
        let mut hashed_keys = HashSet::new();
        hashed_keys.insert("js/main.js".to_string());
        let opts = ManifestOptions {
            cdn_base_url: "https://cdn.example.com".to_string(),
            version: "1.0.0".to_string(),
            build_time: None,
            hashed_keys,
        };
        let manifest = build_manifest(&assets, &opts).unwrap();
        let entry = &manifest.assets["js/main.js"];
        assert_eq!(entry.url, "https://cdn.example.com/js/main.abcd1234.js");
    }

    #[test]
    fn rewrite_urls_to_cdn_rewrites_relative_urls() {
        use std::collections::HashMap as HM;
        use s3d_types::manifest::{AssetEntry, DeployManifest};

        let mut assets = HM::new();
        assets.insert("app.js".to_string(), AssetEntry {
            url: "/app.abcd1234.js".to_string(),
            size: 10,
            hash: "abcd1234".to_string(),
            content_type: "application/javascript".to_string(),
            dependencies: None,
        });
        assets.insert("style.css".to_string(), AssetEntry {
            url: "/style.css".to_string(),
            size: 5,
            hash: "00000000".to_string(),
            content_type: "text/css".to_string(),
            dependencies: None,
        });
        let mut manifest = DeployManifest {
            schema_version: 1,
            version: "1.0.0".to_string(),
            build_time: "2026-03-20T00:00:00Z".to_string(),
            assets,
            strategies: HM::new(),
        };

        rewrite_urls_to_cdn(&mut manifest, "https://cdn.example.com");

        assert_eq!(manifest.assets["app.js"].url, "https://cdn.example.com/app.abcd1234.js");
        assert_eq!(manifest.assets["style.css"].url, "https://cdn.example.com/style.css");
    }

    #[test]
    fn rewrite_urls_to_cdn_is_idempotent() {
        use std::collections::HashMap as HM;
        use s3d_types::manifest::{AssetEntry, DeployManifest};

        let mut assets = HM::new();
        assets.insert("app.js".to_string(), AssetEntry {
            url: "https://cdn.example.com/app.abcd1234.js".to_string(),
            size: 10,
            hash: "abcd1234".to_string(),
            content_type: "application/javascript".to_string(),
            dependencies: None,
        });
        let mut manifest = DeployManifest {
            schema_version: 1,
            version: "1.0.0".to_string(),
            build_time: "2026-03-20T00:00:00Z".to_string(),
            assets,
            strategies: HM::new(),
        };

        // すでに絶対 URL → 書き換えなし（冪等性）
        rewrite_urls_to_cdn(&mut manifest, "https://cdn.example.com");
        assert_eq!(manifest.assets["app.js"].url, "https://cdn.example.com/app.abcd1234.js");
    }
}
