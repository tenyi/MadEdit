//! 字元編碼偵測、解碼與編碼

use encoding_rs::Encoding;

use super::model::NewLineFormat;

/// 選單中常用編碼（依原版 MadEdit 常見順序）
pub const COMMON_ENCODINGS: &[&str] = &[
    "UTF-8",
    "UTF-16LE",
    "UTF-16BE",
    "Big5",
    "GBK",
    "GB18030",
    "Shift_JIS",
    "EUC-JP",
    "EUC-KR",
    "windows-1252",
];

/// encoding_rs 支援的全部編碼（對應原版「所有編碼」子選單）
pub const ALL_ENCODINGS: &[&str] = &[
    "UTF-8",
    "UTF-16LE",
    "UTF-16BE",
    "Big5",
    "GBK",
    "GB18030",
    "Shift_JIS",
    "EUC-JP",
    "ISO-2022-JP",
    "EUC-KR",
    "IBM866",
    "ISO-8859-2",
    "ISO-8859-3",
    "ISO-8859-4",
    "ISO-8859-5",
    "ISO-8859-6",
    "ISO-8859-7",
    "ISO-8859-8",
    "ISO-8859-8-I",
    "ISO-8859-10",
    "ISO-8859-13",
    "ISO-8859-14",
    "ISO-8859-15",
    "ISO-8859-16",
    "KOI8-R",
    "KOI8-U",
    "macintosh",
    "windows-874",
    "windows-1250",
    "windows-1251",
    "windows-1252",
    "windows-1253",
    "windows-1254",
    "windows-1255",
    "windows-1256",
    "windows-1257",
    "windows-1258",
    "x-mac-cyrillic",
];

/// 依名稱取得編碼（不分大小寫，接受 WHATWG 別名）
pub fn encoding_for_name(name: &str) -> Option<&'static Encoding> {
    Encoding::for_label(name.trim().as_bytes())
}

/// 該編碼的 BOM 位元組（無 BOM 概念的編碼回傳空切片）
pub fn bom_bytes(enc: &'static Encoding) -> &'static [u8] {
    if enc == encoding_rs::UTF_8 {
        &[0xEF, 0xBB, 0xBF]
    } else if enc == encoding_rs::UTF_16LE {
        &[0xFF, 0xFE]
    } else if enc == encoding_rs::UTF_16BE {
        &[0xFE, 0xFF]
    } else {
        &[]
    }
}

/// 編碼是否支援 BOM
pub fn supports_bom(enc: &'static Encoding) -> bool {
    !bom_bytes(enc).is_empty()
}

/// 解碼結果
#[derive(Debug)]
pub struct Decoded {
    pub text: String,
    pub encoding: &'static Encoding,
    pub has_bom: bool,
    /// 是否有無法解碼的位元組（已以 U+FFFD 取代）
    pub had_errors: bool,
}

/// 以指定編碼解碼（BOM 只在與編碼相符時去除）
pub fn decode_with(bytes: &[u8], enc: &'static Encoding) -> Decoded {
    let has_bom = matches!(Encoding::for_bom(bytes), Some((bom_enc, _)) if bom_enc == enc);
    let body = if has_bom { &bytes[bom_bytes(enc).len()..] } else { bytes };
    let (text, had_errors) = enc.decode_without_bom_handling(body);
    Decoded {
        text: text.into_owned(),
        encoding: enc,
        has_bom,
        had_errors,
    }
}

/// 判斷資料是否像二進位檔（含 NUL 且不像 UTF-16 文字）
pub fn looks_binary(bytes: &[u8]) -> bool {
    let sample = &bytes[..bytes.len().min(64 * 1024)];
    if Encoding::for_bom(sample).is_some() || guess_utf16(sample).is_some() {
        return false;
    }
    sample.contains(&0)
}

/// 無 BOM 的 UTF-16 猜測：ASCII 文字在 UTF-16 中會有大量位於固定奇偶位置的 0
fn guess_utf16(sample: &[u8]) -> Option<&'static Encoding> {
    if sample.len() < 4 {
        return None;
    }
    let pairs = sample.len() / 2;
    let even_zero = sample.iter().step_by(2).filter(|&&b| b == 0).count();
    let odd_zero = sample.iter().skip(1).step_by(2).filter(|&&b| b == 0).count();
    if odd_zero * 10 > pairs * 4 && even_zero * 50 < pairs {
        Some(encoding_rs::UTF_16LE)
    } else if even_zero * 10 > pairs * 4 && odd_zero * 50 < pairs {
        Some(encoding_rs::UTF_16BE)
    } else {
        None
    }
}

/// 自動偵測編碼並解碼
///
/// 順序：BOM → （強制預設編碼）→ 無 BOM 的 UTF-16 → 嚴格 UTF-8 → 內容偵測（Big5/GBK/
/// Shift_JIS 等）→ fallback 編碼 → 預設編碼 → 常見東亞編碼中第一個可無誤解碼者 → fallback 編碼（含錯誤）。
///
/// `default` 是新文件與「強制使用預設編碼」採用的編碼；`fallback` 只在無法自動判斷時才派上用場。
pub fn detect_and_decode(bytes: &[u8], default: &'static Encoding, fallback: &'static Encoding, enforce_default: bool) -> Decoded {
    if let Some((enc, _)) = Encoding::for_bom(bytes) {
        return decode_with(bytes, enc);
    }
    if enforce_default {
        return decode_with(bytes, default);
    }
    if let Some(enc) = guess_utf16(&bytes[..bytes.len().min(64 * 1024)]) {
        let d = decode_with(bytes, enc);
        if !d.had_errors {
            return d;
        }
    }
    if let Some(text) = encoding_rs::UTF_8.decode_without_bom_handling_and_without_replacement(bytes) {
        return Decoded {
            text: text.into_owned(),
            encoding: encoding_rs::UTF_8,
            has_bom: false,
            had_errors: false,
        };
    }
    // 非 UTF-8：依內容猜測 GBK、Big5、Shift_JIS 等，避免先以錯誤的備援編碼解碼而亂碼。
    let sample = &bytes[..bytes.len().min(64 * 1024)];
    let mut detector = chardetng::EncodingDetector::new();
    detector.feed(sample, true);
    let guessed = detector.guess(None, false);
    if guessed != encoding_rs::UTF_8 && guessed != encoding_rs::UTF_16LE && guessed != encoding_rs::UTF_16BE {
        let d = decode_with(bytes, guessed);
        if !d.had_errors {
            return d;
        }
    }
    let candidates = [
        fallback,
        default,
        encoding_rs::BIG5,
        encoding_rs::GB18030,
        encoding_rs::SHIFT_JIS,
        encoding_rs::EUC_KR,
        encoding_rs::EUC_JP,
    ];
    for enc in candidates {
        if enc == encoding_rs::UTF_8 {
            continue;
        }
        let d = decode_with(bytes, enc);
        if !d.had_errors {
            return d;
        }
    }
    decode_with(bytes, fallback)
}

/// 編碼失敗資訊：目標編碼無法表示的字元
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnmappableChars {
    pub count: usize,
    /// 前幾個無法表示的字元（供錯誤訊息顯示）
    pub samples: Vec<char>,
}

/// 依編碼將文字轉為位元組（UTF-16 自行處理，因 encoding_rs 的編碼器會輸出 UTF-8）
///
/// 目標編碼無法表示某些字元時回傳 Err，不會像 encoding_rs 預設行為那樣
/// 默默寫入 `&#NNNN;` 形式的數字字元參照。
pub fn encode_text(text: &str, enc: &'static Encoding, with_bom: bool) -> Result<Vec<u8>, UnmappableChars> {
    let mut out = Vec::with_capacity(text.len() + 3);
    if with_bom {
        out.extend_from_slice(bom_bytes(enc));
    }
    if enc == encoding_rs::UTF_16LE {
        out.extend(text.encode_utf16().flat_map(|u| u.to_le_bytes()));
    } else if enc == encoding_rs::UTF_16BE {
        out.extend(text.encode_utf16().flat_map(|u| u.to_be_bytes()));
    } else if enc == encoding_rs::UTF_8 {
        out.extend_from_slice(text.as_bytes());
    } else {
        let (encoded, _, had_errors) = enc.encode(text);
        if had_errors {
            return Err(find_unmappable(text, enc));
        }
        out.extend_from_slice(&encoded);
    }
    Ok(out)
}

/// 不檢查錯誤的編碼（用於統計位元組數等只需估算的情境）
pub fn encoded_len(text: &str, enc: &'static Encoding) -> usize {
    if enc == encoding_rs::UTF_16LE || enc == encoding_rs::UTF_16BE {
        text.encode_utf16().count() * 2
    } else if enc == encoding_rs::UTF_8 {
        text.len()
    } else {
        enc.encode(text).0.len()
    }
}

/// 找出目標編碼無法表示的字元
fn find_unmappable(text: &str, enc: &'static Encoding) -> UnmappableChars {
    let mut count = 0;
    let mut samples = Vec::new();
    let mut buf = [0u8; 4];
    for c in text.chars() {
        let (_, _, err) = enc.encode(c.encode_utf8(&mut buf));
        if err {
            count += 1;
            if samples.len() < 8 && !samples.contains(&c) {
                samples.push(c);
            }
        }
    }
    UnmappableChars { count, samples }
}

/// 換行偵測結果
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NewlineInfo {
    /// 出現最多的換行格式（無換行時為 None）
    pub dominant: Option<NewLineFormat>,
    /// 是否混用多種換行格式
    pub mixed: bool,
}

/// 於解碼後的文字上統計換行格式（不受 UTF-16 等多位元組編碼影響）
pub fn detect_newlines(text: &str) -> NewlineInfo {
    let (mut crlf, mut lf, mut cr) = (0usize, 0usize, 0usize);
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\r' if bytes.get(i + 1) == Some(&b'\n') => {
                crlf += 1;
                i += 1;
            }
            b'\r' => cr += 1,
            b'\n' => lf += 1,
            _ => {}
        }
        i += 1;
    }
    let kinds = [crlf, lf, cr].iter().filter(|&&n| n > 0).count();
    let dominant = if kinds == 0 {
        None
    } else if crlf >= lf && crlf >= cr {
        Some(NewLineFormat::Dos)
    } else if lf >= cr {
        Some(NewLineFormat::Unix)
    } else {
        Some(NewLineFormat::Mac)
    };
    NewlineInfo { dominant, mixed: kinds > 1 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_bom_utf8_and_fallback() {
        let d = detect_and_decode(&[0xFF, 0xFE, b'H', 0, b'i', 0], encoding_rs::UTF_8, encoding_rs::BIG5, false);
        assert_eq!(d.encoding, encoding_rs::UTF_16LE);
        assert!(d.has_bom);
        assert_eq!(d.text, "Hi");

        let d = detect_and_decode("中文".as_bytes(), encoding_rs::UTF_8, encoding_rs::BIG5, false);
        assert_eq!(d.encoding, encoding_rs::UTF_8);

        let (big5, _, _) = encoding_rs::BIG5.encode("繁體中文");
        let d = detect_and_decode(&big5, encoding_rs::UTF_8, encoding_rs::BIG5, false);
        assert_eq!(d.encoding, encoding_rs::BIG5);
        assert_eq!(d.text, "繁體中文");
        assert!(!d.had_errors);
    }

    #[test]
    fn test_fallback_encoding_used_only_for_non_utf8() {
        let (gbk, _, _) = encoding_rs::GBK.encode("简体中文测试");
        let d = detect_and_decode(&gbk, encoding_rs::UTF_8, encoding_rs::GBK, false);
        assert_eq!(d.encoding, encoding_rs::GBK);
        assert_eq!(d.text, "简体中文测试");

        let d = detect_and_decode("中文".as_bytes(), encoding_rs::UTF_8, encoding_rs::GBK, false);
        assert_eq!(d.encoding, encoding_rs::UTF_8);
    }

    #[test]
    fn test_auto_detects_gbk_even_with_big5_fallback() {
        // 之前 fallback（Big5）先解碼，GBK 檔案常被判成 Big5 而亂碼；內容偵測要選到 GBK。
        let (gbk, _, _) = encoding_rs::GBK.encode("D:项目路径 简体中文测试");
        let d = detect_and_decode(&gbk, encoding_rs::UTF_8, encoding_rs::BIG5, false);
        assert_eq!(d.text, "D:项目路径 简体中文测试");
        assert!(!d.had_errors);
        assert!(d.encoding == encoding_rs::GBK || d.encoding == encoding_rs::GB18030, "detected {:?}", d.encoding.name());
    }

    #[test]
    fn test_utf16_without_bom_guess() {
        let bytes: Vec<u8> = "hello world".encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        let d = detect_and_decode(&bytes, encoding_rs::UTF_8, encoding_rs::BIG5, false);
        assert_eq!(d.encoding, encoding_rs::UTF_16LE);
        assert_eq!(d.text, "hello world");
        assert!(!looks_binary(&bytes));
        assert!(looks_binary(&[1, 2, 0, 0, 0, 5, 6, 0xFF, 0, 0x10, 0x20, 0]));
    }

    #[test]
    fn test_encode_reports_unmappable_chars() {
        let err = encode_text("A中文", encoding_rs::WINDOWS_1252, false).unwrap_err();
        assert_eq!(err.count, 2);
        assert_eq!(err.samples, vec!['中', '文']);
        assert_eq!(encode_text("Hi", encoding_rs::UTF_16BE, true).unwrap(), vec![0xFE, 0xFF, 0, b'H', 0, b'i']);
    }

    #[test]
    fn test_newline_detection_on_decoded_text() {
        let info = detect_newlines("a\r\nb\r\nc\nd");
        assert_eq!(info.dominant, Some(NewLineFormat::Dos));
        assert!(info.mixed);
        let info = detect_newlines("a\rb");
        assert_eq!(info.dominant, Some(NewLineFormat::Mac));
        assert!(!info.mixed);
        assert_eq!(detect_newlines("abc").dominant, None);
    }
}
