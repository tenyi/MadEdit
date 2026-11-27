//! 收藏群組：把常用檔案分組存放，供側邊面板快速開啟（不屬於文件內容，單獨保存於設定檔）

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// 一個收藏項目：檔案路徑與可選的顯示名稱。
///
/// 重新命名只改變面板上顯示的文字，不會移動或改名磁碟上的檔案。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FavoriteItem {
    pub path: PathBuf,
    #[serde(default)]
    pub name: Option<String>,
}

impl FavoriteItem {
    pub fn new(path: PathBuf) -> Self {
        Self { path: clean_path(path), name: None }
    }

    /// 檔名；路徑結尾沒有檔名時退回完整路徑
    pub fn file_name(&self) -> String {
        self.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| self.path.display().to_string())
    }

    /// 面板顯示的文字：自訂名稱優先；否則依「僅顯示名稱」顯示檔名或完整路徑
    pub fn label(&self, names_only: bool) -> String {
        if let Some(name) = self.name.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
            return name.to_string();
        }
        if names_only { self.file_name() } else { self.path.display().to_string() }
    }

    /// 重新命名；空白名稱代表恢復預設顯示（檔名或路徑）
    pub fn rename(&mut self, name: &str) {
        let name = name.trim();
        self.name = (!name.is_empty()).then(|| name.to_string());
    }
}

/// 一個收藏群組：名稱、展開狀態與其中的檔案
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct FavoriteGroup {
    pub name: String,
    /// 面板中是否展開
    pub expanded: bool,
    pub items: Vec<FavoriteItem>,
}

/// Windows 的 canonicalize 會加上 `\\?\` 前綴；收藏顯示與儲存時去掉，仍然指向同一個檔案。
/// UNC 路徑的 `\\?\UNC\server\share` 還原成 `\\server\share`。
/// 去除 Windows 的 verbatim 前綴（`\\?\`、`\\?\UNC\`）；cmd 等外部程式不認得這種路徑
pub(crate) fn clean_path(path: PathBuf) -> PathBuf {
    #[cfg(windows)]
    if let Some(s) = path.to_str() {
        if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{rest}"));
        }
        if let Some(rest) = s.strip_prefix(r"\\?\") {
            return PathBuf::from(rest);
        }
    }
    path
}

/// 兩個路徑是否指向同一個檔案。檔案不存在或無法存取時退回字面比較。
fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

impl FavoriteGroup {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into(), expanded: true, items: Vec::new() }
    }

    pub fn contains(&self, path: &Path) -> bool {
        self.items.iter().any(|item| same_file(&item.path, path))
    }

    /// 加入檔案；已經在群組裡時不重複加入並回傳 false
    pub fn add(&mut self, path: PathBuf) -> bool {
        self.add_item(FavoriteItem::new(path))
    }

    pub fn add_item(&mut self, mut item: FavoriteItem) -> bool {
        item.path = clean_path(item.path);
        if self.contains(&item.path) {
            return false;
        }
        self.items.push(item);
        true
    }

    /// 移除指定項目；索引超出範圍時回傳 false
    pub fn remove(&mut self, index: usize) -> bool {
        if index < self.items.len() {
            self.items.remove(index);
            true
        } else {
            false
        }
    }

    /// 重新命名群組；名稱會去除前後空白
    pub fn rename(&mut self, name: &str) {
        self.name = name.trim().to_string();
    }
}

/// 跨群組移動一個收藏項目（面板拖動排序的資料層）：自訂顯示名跟著項目走，
/// 目標群組已含同一檔案時拒絕並回傳 false；`insertion` 為 `None` 時追加到末尾。
/// 來源群組用 `Vec::remove` 收合，不會留下空位。同群組內排序請用 [`reorder_item`]。
pub fn move_item_between_groups(
    source: &mut FavoriteGroup,
    index: usize,
    target: &mut FavoriteGroup,
    insertion: Option<usize>,
) -> bool {
    let Some(item) = source.items.get(index) else {
        return false;
    };
    if target.contains(&item.path) {
        return false;
    }
    let item = source.items.remove(index);
    let at = insertion.map_or(target.items.len(), |i| i.min(target.items.len()));
    target.items.insert(at, item);
    true
}

/// 群組內排序：把 `from` 位置的項目移動到插入點 `insertion`（懸停列索引 + 上半/下半），
/// 並做死區補償（`from` 與 `from + 1` 的插入點數學上就是原位，回傳 false 不動）。
pub fn reorder_item(group: &mut FavoriteGroup, from: usize, insertion: usize) -> bool {
    let n = group.items.len();
    if from >= n {
        return false;
    }
    let target = insertion.min(n);
    if target == from || target == from + 1 {
        return false;
    }
    let item = group.items.remove(from);
    let to = if from < target { target - 1 } else { target };
    group.items.insert(to, item);
    true
}

/// 群組排序：與 [`reorder_item`] 同一套死區補償，作用於設定檔的群組列表。
/// 換位成功時回傳被拖動群組的新索引。
pub fn reorder_group(groups: &mut Vec<FavoriteGroup>, from: usize, insertion: usize) -> Option<usize> {
    if from >= groups.len() {
        return None;
    }
    let target = insertion.min(groups.len());
    if target == from || target == from + 1 {
        return None;
    }
    let group = groups.remove(from);
    let to = if from < target { target - 1 } else { target };
    groups.insert(to, group);
    Some(to)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_group_adds_each_file_once_and_keeps_the_original_name() {
        let mut group = FavoriteGroup::new("專案");
        assert!(group.add(PathBuf::from("/work/a.txt")));
        assert!(!group.add(PathBuf::from("/work/a.txt")), "同一個檔案不應重複加入");
        assert!(group.add(PathBuf::from("/work/b.txt")));
        assert_eq!(group.items.len(), 2);
        assert_eq!(group.items[0].path, PathBuf::from("/work/a.txt"));
        assert!(group.remove(0));
        assert!(!group.remove(9));
        assert_eq!(group.items.len(), 1);
    }

    #[test]
    fn test_item_label_prefers_rename_then_file_name_or_full_path() {
        let mut item = FavoriteItem::new(PathBuf::from("/work/some/dir/a.txt"));
        assert_eq!(item.label(true), "a.txt");
        assert_eq!(item.label(false), "/work/some/dir/a.txt");
        item.rename("  報價單  ");
        assert_eq!(item.label(true), "報價單");
        assert_eq!(item.label(false), "報價單");
        // 空白名稱恢復預設顯示
        item.rename("   ");
        assert_eq!(item.name, None);
        assert_eq!(item.label(true), "a.txt");
    }

    #[test]
    fn test_group_serde_roundtrip_keeps_order_names_and_fold_state() {
        let mut group = FavoriteGroup::new("常用");
        group.add(PathBuf::from("/a.txt"));
        group.add(PathBuf::from("/b.txt"));
        group.items[1].rename("乙檔案");
        group.expanded = false;
        let text = serde_json::to_string(&group).unwrap();
        let back: FavoriteGroup = serde_json::from_str(&text).unwrap();
        assert_eq!(back, group);
        // 舊版資料沒有 name 或 expanded：使用預設值，不會讓整份設定失效
        let legacy: FavoriteGroup = serde_json::from_str(r#"{"name":"舊群組","items":[{"path":"/a.txt"}]}"#).unwrap();
        assert!(legacy.items[0].name.is_none());
        assert!(!legacy.expanded);
    }

    #[test]
    fn test_rename_trims_whitespace() {
        let mut group = FavoriteGroup::new(" 舊名稱 ");
        group.rename("  新名稱  ");
        assert_eq!(group.name, "新名稱");
    }

    #[test]
    fn test_group_dedupes_the_same_file_spelled_differently() {
        let dir = std::env::temp_dir().join(format!("madedit-favorites-same-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.txt");
        std::fs::write(&file, "a").unwrap();
        let mut group = FavoriteGroup::new("工作");
        assert!(group.add(file.clone()));
        std::fs::create_dir(dir.join("sub")).unwrap();
        let spelled = dir.join("sub").join("..").join("a.txt");
        assert!(!group.add(spelled), "同一個檔案的不同寫法不應重複加入");
        assert_eq!(group.items.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(windows)]
    #[test]
    fn test_windows_verbatim_prefix_is_removed_from_favorite_paths() {
        assert_eq!(FavoriteItem::new(PathBuf::from(r"\\?\C:\work\a.txt")).path, PathBuf::from(r"C:\work\a.txt"));
        assert_eq!(FavoriteItem::new(PathBuf::from(r"\\?\UNC\server\share\a.txt")).path, PathBuf::from(r"\\server\share\a.txt"));
    }

    #[test]
    fn test_move_item_between_groups_keeps_name_and_refuses_duplicates() {
        let mut a = FavoriteGroup::new("甲");
        let mut b = FavoriteGroup::new("乙");
        a.add(PathBuf::from("/1.txt"));
        a.add(PathBuf::from("/2.txt"));
        b.add(PathBuf::from("/3.txt"));
        a.items[1].rename("乙檔案");

        // 跨組：插入到乙組開頭，自訂名稱跟著走
        assert!(move_item_between_groups(&mut a, 1, &mut b, Some(0)));
        assert_eq!(a.items.iter().map(|i| i.file_name()).collect::<Vec<_>>(), vec!["1.txt"]);
        assert_eq!(b.items[0].file_name(), "2.txt");
        assert_eq!(b.items[0].name.as_deref(), Some("乙檔案"));

        // 目標已含同一檔案 → 拒絕，來源不動（造一個兩組共有的路徑）
        let mut x = FavoriteGroup::new("丙");
        let mut y = FavoriteGroup::new("丁");
        x.add(PathBuf::from("/dup.txt"));
        y.add(PathBuf::from("/dup.txt"));
        assert!(!move_item_between_groups(&mut x, 0, &mut y, None));
        assert_eq!(x.items.len(), 1);
        assert_eq!(y.items.len(), 1);

        // insertion None → 追加到末尾
        assert!(move_item_between_groups(&mut a, 0, &mut b, None));
        assert_eq!(b.items.iter().map(|i| i.file_name()).collect::<Vec<_>>(), vec!["2.txt", "3.txt", "1.txt"]);
        assert_eq!(a.items.len(), 0);
    }

    #[test]
    fn test_reorder_helpers_skip_dead_zones_and_compensate_index() {
        let mut g = FavoriteGroup::new("組");
        for name in ["/1.txt", "/2.txt", "/3.txt", "/4.txt"] {
            g.add(PathBuf::from(name));
        }
        // 死區：插入點等於原位（頭頂/腳底）→ false 不動
        assert!(!reorder_item(&mut g, 1, 1));
        assert!(!reorder_item(&mut g, 1, 2));
        // 1 移到 4 之後（插入點 4）→ remove 後補償為 3
        assert!(reorder_item(&mut g, 1, 4));
        assert_eq!(g.items.iter().map(|i| i.file_name()).collect::<Vec<_>>(), vec!["1.txt", "3.txt", "4.txt", "2.txt"]);
        assert!(!reorder_item(&mut g, 9, 0), "索引超出範圍 → false");

        let mut groups = vec![FavoriteGroup::new("A"), FavoriteGroup::new("B"), FavoriteGroup::new("C")];
        // 死區
        assert_eq!(reorder_group(&mut groups, 0, 0), None);
        assert_eq!(reorder_group(&mut groups, 0, 1), None);
        // A 拖到 C 之後
        assert_eq!(reorder_group(&mut groups, 0, 3), Some(2));
        assert_eq!(groups.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(), vec!["B", "C", "A"]);
        assert_eq!(reorder_group(&mut groups, 5, 0), None);
    }
}
