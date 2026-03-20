//! SHA-256 ハッシュ計算とハッシュ付きファイル名生成モジュール
//!
//! `model.glb` → `model.a1b2c3d4.glb` のようなコンテンツハッシュ付きファイル名を生成する。
//! ハッシュ長はデフォルト 8 文字。

use std::io::Read;
use std::path::PathBuf;

use hex::encode as hex_encode;
use s3d_types::asset::{CollectedAsset, HashedAsset};
use sha2::{Digest, Sha256};
use thiserror::Error;

/// hash モジュールのエラー型
#[derive(Debug, Error)]
pub enum HashError {
    #[error("I/O error reading `{path}`: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// デフォルトのハッシュ文字数（SHA-256 hex の先頭 N 文字）
pub const DEFAULT_HASH_LENGTH: usize = 8;

/// ファイルの全内容から SHA-256 ダイジェストを計算し、hex 文字列で返す。
pub fn sha256_file(path: &std::path::Path) -> Result<String, HashError> {
    let mut file = std::fs::File::open(path).map_err(|source| HashError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = file.read(&mut buf).map_err(|source| HashError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex_encode(hasher.finalize()))
}

/// `key` のパスにハッシュを挿入したハッシュ付きキーを生成する。
///
/// 例: `"assets/model.glb"`, hash=`"a1b2c3d4"` → `"assets/model.a1b2c3d4.glb"`
///
/// 拡張子がない場合はキーの末尾に `.<hash>` を付与する。
pub fn insert_hash_into_key(key: &str, hash: &str) -> String {
    // パス区切りを保持しながらファイル名部分だけ加工する
    let (dir, filename) = match key.rfind('/') {
        Some(pos) => (&key[..=pos], &key[pos + 1..]),
        None => ("", key),
    };

    let new_filename = if let Some(dot) = filename.rfind('.') {
        let stem = &filename[..dot];
        let ext = &filename[dot..]; // "." を含む
        format!("{stem}.{hash}{ext}")
    } else {
        format!("{filename}.{hash}")
    };

    format!("{dir}{new_filename}")
}

/// [`CollectedAsset`] のリストに SHA-256 ハッシュを付与し [`HashedAsset`] のリストを返す。
///
/// `hash_length` には使用するハッシュ文字数を指定する（デフォルト: [`DEFAULT_HASH_LENGTH`]）。
pub fn hash_assets(
    assets: &[CollectedAsset],
    hash_length: usize,
) -> Result<Vec<HashedAsset>, HashError> {
    let mut result = Vec::with_capacity(assets.len());
    for asset in assets {
        let full_hash = sha256_file(&asset.absolute_path)?;
        let hash = full_hash[..hash_length.min(full_hash.len())].to_string();

        let hashed_key = insert_hash_into_key(&asset.key, &hash);
        // hashed_filename はパスの最後の成分のみ
        let hashed_filename = hashed_key
            .rsplit('/')
            .next()
            .unwrap_or(&hashed_key)
            .to_string();

        result.push(HashedAsset {
            key: asset.key.clone(),
            absolute_path: asset.absolute_path.clone(),
            size: asset.size,
            hash,
            hashed_filename,
            hashed_key,
        });
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn sha256_known_content() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("hello.txt");
        fs::write(&path, b"hello").unwrap();
        // echo -n "hello" | sha256sum = 2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824
        let hash = sha256_file(&path).unwrap();
        assert_eq!(
            hash,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[test]
    fn insert_hash_with_extension() {
        assert_eq!(
            insert_hash_into_key("assets/model.glb", "a1b2c3d4"),
            "assets/model.a1b2c3d4.glb"
        );
    }

    #[test]
    fn insert_hash_without_extension() {
        assert_eq!(
            insert_hash_into_key("assets/model", "a1b2c3d4"),
            "assets/model.a1b2c3d4"
        );
    }

    #[test]
    fn insert_hash_root_file() {
        assert_eq!(
            insert_hash_into_key("index.html", "deadbeef"),
            "index.deadbeef.html"
        );
    }

    #[test]
    fn insert_hash_dotfile() {
        // ".gitignore" のようなドットファイル: rfind('.') が 0 を返す
        // stem="" ext=".gitignore" → ".abcd1234.gitignore"
        // 実用上 dotfile はアセット対象外になることが多いが、動作を文書化しておく
        assert_eq!(
            insert_hash_into_key(".gitignore", "abcd1234"),
            ".abcd1234.gitignore"
        );
    }

    #[test]
    fn hash_assets_produces_correct_key() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("chunk.js");
        fs::write(&path, b"const x=1;").unwrap();

        let collected = vec![CollectedAsset {
            key: "js/chunk.js".to_string(),
            absolute_path: path,
            size: 10,
        }];

        let hashed = hash_assets(&collected, 8).unwrap();
        assert_eq!(hashed.len(), 1);
        let h = &hashed[0];
        assert_eq!(h.key, "js/chunk.js");
        assert_eq!(h.hash.len(), 8);
        assert!(h.hashed_key.starts_with("js/chunk."));
        assert!(h.hashed_key.ends_with(".js"));
        assert_eq!(h.hashed_filename, h.hashed_key.rsplit('/').next().unwrap());
    }

    #[test]
    fn hash_length_is_respected() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("a.bin");
        fs::write(&path, b"data").unwrap();

        let collected = vec![CollectedAsset {
            key: "a.bin".to_string(),
            absolute_path: path,
            size: 4,
        }];

        for len in [4, 8, 16] {
            let hashed = hash_assets(&collected, len).unwrap();
            assert_eq!(hashed[0].hash.len(), len);
        }
    }
}
