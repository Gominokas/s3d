//! マニフェスト生成モジュール
//!
//! [`HashedAsset`] のリストと CDN base URL・バージョン文字列から
//! [`DeployManifest`] を構築し JSON にシリアライズする。
//!
//! MIME タイプは拡張子から推定する。
//! glTF アセット (`.gltf` / `.glb`) の依存関係（`.bin` / テクスチャ）は
//! ファイル内容を解析して自動的にマッピングする。

use std::collections::HashMap;
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
    /// CDN の base URL（末尾スラッシュなし）
    pub cdn_base_url: String,
    /// デプロイバージョン（例: `"1.0.0"`）
    pub version: String,
    /// ビルド日時（RFC3339）。`None` の場合は現在時刻の代わりに空文字を使う
    pub build_time: Option<String>,
}

/// [`HashedAsset`] のリストから [`DeployManifest`] を構築する。
///
/// - URL = `{cdn_base_url}/{hashed_key}`
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
        let url = format!("{}/{}", base, asset.hashed_key);
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
    })
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

        let opts = ManifestOptions {
            cdn_base_url: "https://cdn.example.com".to_string(),
            version: "1.0.0".to_string(),
            build_time: Some("2026-03-20T00:00:00Z".to_string()),
        };

        let manifest = build_manifest(&assets, &opts).unwrap();
        assert_eq!(manifest.schema_version, 1);
        assert_eq!(manifest.version, "1.0.0");
        // キーは元のパス（ハッシュなし）
        assert!(manifest.assets.contains_key("js/main.js"));

        let entry = &manifest.assets["js/main.js"];
        assert_eq!(entry.url, "https://cdn.example.com/js/main.abcd1234.js");
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

        let opts = ManifestOptions {
            cdn_base_url: "https://cdn.example.com".to_string(),
            version: "1.0.0".to_string(),
            build_time: None,
        };

        let manifest = build_manifest(&assets, &opts).unwrap();
        // gltf エントリのキーは元のパス（ハッシュなし）
        let gltf_entry = manifest.assets.get("scene.gltf").unwrap();
        let deps = gltf_entry.dependencies.as_ref().unwrap();
        // 依存先の URL は hashed_key ベース
        assert!(deps.contains(&"buffer.bbbb0002.bin".to_string()));
    }
}
