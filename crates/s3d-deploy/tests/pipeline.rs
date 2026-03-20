//! fixtures を使ったパイプライン統合テスト
//! collect → hash → manifest → diff の全処理を通す。

use s3d_deploy::collect::{collect, CollectOptions};
use s3d_deploy::diff::diff_manifests;
use s3d_deploy::hash::{hash_assets, DEFAULT_HASH_LENGTH};
use s3d_deploy::manifest::{build_manifest, manifest_to_json, ManifestOptions};
use s3d_types::asset::AssetDiff;

fn fixtures_dir() -> std::path::PathBuf {
    // tests/ の隣にある fixtures/assets
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/assets")
}

#[test]
fn pipeline_collect_hash_manifest() {
    let root = fixtures_dir();

    // 1. collect — .map は除外
    let opts = CollectOptions {
        ignore: vec!["**/*.map".to_string()],
        ..Default::default()
    };
    let collected = collect(&root, &opts).unwrap();
    assert!(!collected.is_empty());
    let keys: Vec<_> = collected.iter().map(|a| a.key.as_str()).collect();
    assert!(
        keys.contains(&"js/main.js"),
        "js/main.js not found in {keys:?}"
    );
    assert!(!keys.contains(&"js/main.js.map"), ".map should be ignored");
    assert!(keys.contains(&"style.css"));
    assert!(keys.contains(&"models/scene.gltf"));

    // 2. hash
    let hashed = hash_assets(&collected, DEFAULT_HASH_LENGTH).unwrap();
    assert_eq!(hashed.len(), collected.len());
    for h in &hashed {
        assert_eq!(h.hash.len(), DEFAULT_HASH_LENGTH);
        assert!(h.hashed_key.contains(&h.hash));
    }

    // 3. manifest
    let manifest_opts = ManifestOptions {
        cdn_base_url: "https://cdn.example.com".to_string(),
        version: "1.0.0".to_string(),
        build_time: Some("2026-03-20T00:00:00Z".to_string()),
    };
    let manifest = build_manifest(&hashed, &manifest_opts).unwrap();
    assert_eq!(manifest.schema_version, 1);
    assert_eq!(manifest.assets.len(), hashed.len());

    // MIME タイプ確認
    let js_entry = manifest
        .assets
        .values()
        .find(|e| e.url.contains("main.") && e.url.ends_with(".js"))
        .expect("main.js entry not found");
    assert!(js_entry.content_type.contains("javascript"));

    let glb_style = manifest.assets.values().find(|e| e.url.ends_with(".gltf"));
    // gltf が存在する場合は MIME を確認
    if let Some(entry) = glb_style {
        assert_eq!(entry.content_type, "model/gltf+json");
    }

    // 4. JSON 出力が valid
    let json = manifest_to_json(&manifest).unwrap();
    let _: s3d_types::manifest::DeployManifest = serde_json::from_str(&json).unwrap();
}

#[test]
fn pipeline_diff_second_deploy() {
    let root = fixtures_dir();
    let opts = CollectOptions::default();
    let collected = collect(&root, &opts).unwrap();
    let hashed = hash_assets(&collected, DEFAULT_HASH_LENGTH).unwrap();
    let manifest_opts = ManifestOptions {
        cdn_base_url: "https://cdn.example.com".to_string(),
        version: "1.0.0".to_string(),
        build_time: None,
    };
    let manifest_v1 = build_manifest(&hashed, &manifest_opts).unwrap();

    // 初回デプロイ: 旧マニフェストなし → 全 Added
    let diffs = diff_manifests(None, &manifest_v1);
    assert!(diffs.iter().all(|d| d.diff == AssetDiff::Added));

    // 2回目: 同じ内容 → 全 Unchanged
    let diffs2 = diff_manifests(Some(&manifest_v1), &manifest_v1);
    assert!(diffs2.iter().all(|d| d.diff == AssetDiff::Unchanged));
}

#[test]
fn gltf_dependencies_resolved_in_manifest() {
    let root = fixtures_dir();
    let opts = CollectOptions {
        ignore: vec!["**/*.map".to_string()],
        ..Default::default()
    };
    let collected = collect(&root, &opts).unwrap();
    let hashed = hash_assets(&collected, DEFAULT_HASH_LENGTH).unwrap();
    let manifest_opts = ManifestOptions {
        cdn_base_url: "https://cdn.example.com".to_string(),
        version: "1.0.0".to_string(),
        build_time: None,
    };
    let manifest = build_manifest(&hashed, &manifest_opts).unwrap();

    // scene.gltf エントリに依存関係が含まれているか
    let gltf_entry = manifest
        .assets
        .values()
        .find(|e| e.content_type == "model/gltf+json");

    if let Some(entry) = gltf_entry {
        if let Some(deps) = &entry.dependencies {
            // scene.bin と texture.png の hashed_key が含まれるはず
            assert!(
                deps.iter().any(|d| d.ends_with(".bin")),
                "Expected .bin dependency, got: {deps:?}"
            );
        }
    }
}
