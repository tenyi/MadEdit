pub mod chinese;
mod chinese_phrases;
mod chinese_tables;
pub mod dprint;
pub mod document;
pub mod editor;
pub mod encoding;
pub mod hexbuf;
pub mod model;
pub mod piece_table;
pub mod search;
pub mod settings;
pub mod spell;
pub mod syntax;
pub mod textops;

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
