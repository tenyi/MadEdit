pub mod chinese;
pub mod compare;
mod chinese_phrases;
mod chinese_tables;
pub mod dprint;
pub mod document;
pub mod editor;
pub mod favorites;
pub mod encoding;
pub mod fold;
pub mod hexbuf;
pub mod model;
pub mod piece_table;
pub mod search;
pub mod settings;
pub mod spell;
pub mod syntax;
pub mod textops;

use std::path::Path;

/// A path as the user should see and copy it: on Windows the verbatim prefix that
/// `canonicalize` adds (`\\?\D:\…`, `\\?\UNC\server\share`) is dropped, because neither users
/// nor Explorer or `cmd` expect it. Other platforms return the path unchanged.
pub fn display_path(path: &Path) -> String {
    favorites::clean_path(path.to_path_buf()).display().to_string()
}

/// 逐 8 位元組混合的內容雜湊（FxHash 變體）
///
/// 供換行、拼字與語法快取偵測行內容是否變更；不需要抗碰撞，只求快。
pub fn content_hash(text: &str) -> u64 {
    const MUL: u64 = 0x517c_c1b7_2722_0a95;
    let mut hash: u64 = 0x9e37_79b9_7f4a_7c15;
    let bytes = text.as_bytes();
    let (chunks, rem) = bytes.as_chunks::<8>();
    for c in chunks {
        hash = (hash.rotate_left(5) ^ u64::from_le_bytes(*c)).wrapping_mul(MUL);
    }
    if !rem.is_empty() {
        let mut tail = [0u8; 8];
        tail[..rem.len()].copy_from_slice(rem);
        hash = (hash.rotate_left(5) ^ u64::from_le_bytes(tail)).wrapping_mul(MUL);
    }
    hash ^ (bytes.len() as u64).wrapping_mul(MUL)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// 顯示／複製用的路徑：Windows 上不帶 canonicalize 的 `\\?\` 前綴，
    /// 其他平台原樣傳回。
    #[test]
    fn test_display_path_has_no_windows_verbatim_prefix() {
        let plain = PathBuf::from(r"D:\work\a.txt");
        assert_eq!(display_path(&plain), plain.display().to_string());
        #[cfg(windows)]
        {
            assert_eq!(display_path(&PathBuf::from(r"\\?\D:\work\a.txt")), plain.display().to_string());
            assert_eq!(display_path(&PathBuf::from(r"\\?\UNC\server\share\a.txt")), r"\\server\share\a.txt");
        }
    }
}
