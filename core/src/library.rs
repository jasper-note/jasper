//! 内存索引层：把一批条目原始内容解析后组织成可查询的库（纯计算，无 IO）。
//!
//! 只读、构建一次即不可变。从存储拉取 + 增量缓存的协调放在 server 的 `indexer`，
//! 它拿到原始内容后调用 `Library::from_contents`。本模块不引入线程/IO，可编译到 WASM。

use crate::model::*;
use crate::parser;
use std::collections::{HashMap, HashSet};

#[derive(Default)]
pub struct Library {
    pub folders: HashMap<String, Folder>,
    pub notes: HashMap<String, Note>,
    pub resources: HashMap<String, Resource>,
    pub tags: HashMap<String, Tag>,
    pub note_tags: Vec<NoteTag>,

    /// folder_id -> 该笔记本下的笔记 id（直属，不含子笔记本）
    notes_by_folder: HashMap<String, Vec<String>>,
    /// parent folder_id -> 子笔记本 id（根用空串 ""）
    child_folders: HashMap<String, Vec<String>>,
    /// tag_id -> 打了该标签且仍存在的笔记 id（去重；据 note_tags 关联表构建）
    notes_by_tag: HashMap<String, Vec<String>>,
    /// resource_id -> 引用它的笔记 id（据笔记正文里的 `:/<id>` 扫描构建；用于资源访问控制的
    /// resource→note→folder 权限链路，见 server::api::resource）
    resource_notes: HashMap<String, Vec<String>>,

    /// 笔记的原始 .md 内容（用于保存时保留元数据，避免重新拉取）
    raw_notes: HashMap<String, String>,
}

#[derive(Debug, Default, Clone)]
pub struct BuildStats {
    pub notes: usize,
    pub folders: usize,
    pub resources: usize,
    pub tags: usize,
    pub note_tags: usize,
    pub others: usize,
    pub encrypted: usize,
    pub errors: usize,
    /// 命中增量缓存、免去拉取的条目数。
    pub cached: usize,
    /// 本轮实际从数据源拉取的条目数。
    pub fetched: usize,
}

impl Library {
    /// 把一批原始 .md 内容解析、分类、建索引为 Library，返回类型计数 + 解析错误数。
    /// 拉取（并行/缓存）由调用方负责；这里只做纯解析与索引，便于编译到 WASM。
    pub fn from_contents(contents: Vec<String>) -> (Library, BuildStats) {
        let mut lib = Library::default();
        let mut stats = BuildStats::default();

        // 顺序解析（核心库不引入线程）；保留笔记原始内容用于写回。
        let parsed: Vec<Option<(String, RawItem)>> = contents
            .into_iter()
            .map(|content| parser::parse_item(&content).ok().map(|raw| (content, raw)))
            .collect();

        // 分类，构建索引
        for item in parsed {
            let (content, raw) = match item {
                Some(x) => x,
                None => {
                    stats.errors += 1;
                    continue;
                }
            };
            if raw.is_encrypted() {
                stats.encrypted += 1;
                continue;
            }
            match raw.item_type() {
                ItemType::Note => match parser::to_note(&raw) {
                    Ok(n) => {
                        lib.raw_notes.insert(n.id.clone(), content);
                        lib.notes.insert(n.id.clone(), n);
                        stats.notes += 1;
                    }
                    Err(_) => stats.errors += 1,
                },
                ItemType::Folder => match parser::to_folder(&raw) {
                    Ok(f) => {
                        lib.folders.insert(f.id.clone(), f);
                        stats.folders += 1;
                    }
                    Err(_) => stats.errors += 1,
                },
                ItemType::Resource => match parser::to_resource(&raw) {
                    Ok(r) => {
                        lib.resources.insert(r.id.clone(), r);
                        stats.resources += 1;
                    }
                    Err(_) => stats.errors += 1,
                },
                ItemType::Tag => match parser::to_tag(&raw) {
                    Ok(t) => {
                        lib.tags.insert(t.id.clone(), t);
                        stats.tags += 1;
                    }
                    Err(_) => stats.errors += 1,
                },
                ItemType::NoteTag => match parser::to_note_tag(&raw) {
                    Ok(nt) => {
                        lib.note_tags.push(nt);
                        stats.note_tags += 1;
                    }
                    Err(_) => stats.errors += 1,
                },
                ItemType::Other => stats.others += 1,
            }
        }

        lib.build_indexes();
        (lib, stats)
    }

    fn build_indexes(&mut self) {
        let mut notes_by_folder: HashMap<String, Vec<String>> = HashMap::new();
        for n in self.notes.values() {
            notes_by_folder
                .entry(n.parent_id.clone())
                .or_default()
                .push(n.id.clone());
        }
        let mut child_folders: HashMap<String, Vec<String>> = HashMap::new();
        for f in self.folders.values() {
            child_folders
                .entry(f.parent_id.clone())
                .or_default()
                .push(f.id.clone());
        }
        // 标签 → 笔记：只收仍存在的笔记（note_tag 可能悬挂到已删笔记），按 (tag,note) 去重。
        let mut notes_by_tag: HashMap<String, Vec<String>> = HashMap::new();
        let mut seen: HashMap<String, HashSet<String>> = HashMap::new();
        for nt in &self.note_tags {
            if !self.notes.contains_key(&nt.note_id) {
                continue;
            }
            if seen.entry(nt.tag_id.clone()).or_default().insert(nt.note_id.clone()) {
                notes_by_tag.entry(nt.tag_id.clone()).or_default().push(nt.note_id.clone());
            }
        }
        // 资源 → 引用它的笔记：扫每篇笔记正文的 `:/<id>` 引用（每篇笔记内已去重）。
        let mut resource_notes: HashMap<String, Vec<String>> = HashMap::new();
        for n in self.notes.values() {
            for rid in scan_resource_refs(&n.body) {
                resource_notes.entry(rid).or_default().push(n.id.clone());
            }
        }
        self.notes_by_folder = notes_by_folder;
        self.child_folders = child_folders;
        self.notes_by_tag = notes_by_tag;
        self.resource_notes = resource_notes;
    }

    /// 某笔记本（含根 ""）直属笔记数。
    pub fn note_count(&self, folder_id: &str) -> usize {
        self.notes_by_folder.get(folder_id).map(|v| v.len()).unwrap_or(0)
    }

    /// 子笔记本，按标题排序。
    pub fn child_folder_ids_sorted(&self, parent_id: &str) -> Vec<String> {
        let mut ids = self.child_folders.get(parent_id).cloned().unwrap_or_default();
        ids.sort_by(|a, b| {
            let ta = self.folders.get(a).map(|f| f.title.as_str()).unwrap_or("");
            let tb = self.folders.get(b).map(|f| f.title.as_str()).unwrap_or("");
            ta.cmp(tb)
        });
        ids
    }

    /// 某笔记本下的笔记，按更新时间倒序。
    pub fn notes_in_folder_sorted(&self, folder_id: &str) -> Vec<&Note> {
        let mut notes: Vec<&Note> = self
            .notes_by_folder
            .get(folder_id)
            .map(|ids| ids.iter().filter_map(|id| self.notes.get(id)).collect())
            .unwrap_or_default();
        notes.sort_by(|a, b| b.updated_time.cmp(&a.updated_time));
        notes
    }

    /// 某标签下仍存在的笔记数。
    pub fn tag_note_count(&self, tag_id: &str) -> usize {
        self.notes_by_tag.get(tag_id).map(|v| v.len()).unwrap_or(0)
    }

    /// 全部标签，按标题（不区分大小写）排序。
    pub fn tags_sorted(&self) -> Vec<&Tag> {
        let mut tags: Vec<&Tag> = self.tags.values().collect();
        tags.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));
        tags
    }

    /// 打了某标签的笔记，按更新时间倒序（与笔记本视图一致）。
    pub fn notes_with_tag(&self, tag_id: &str) -> Vec<&Note> {
        let mut notes: Vec<&Note> = self
            .notes_by_tag
            .get(tag_id)
            .map(|ids| ids.iter().filter_map(|id| self.notes.get(id)).collect())
            .unwrap_or_default();
        notes.sort_by(|a, b| b.updated_time.cmp(&a.updated_time));
        notes
    }

    /// 按标题查已有标签 id（trim + 不区分大小写，对齐 Joplin `Tag.loadByTitle` 语义）。
    /// 多个同名时取 id 最小者以保证确定性（Joplin 用 created_time，本模型不存该字段）。
    pub fn tag_id_by_title(&self, title: &str) -> Option<String> {
        let key = title.trim().to_lowercase();
        if key.is_empty() {
            return None;
        }
        self.tags
            .values()
            .filter(|t| t.title.trim().to_lowercase() == key)
            .map(|t| &t.id)
            .min()
            .cloned()
    }

    /// 笔记是否已打某标签（据关联表；用于新增去重、对齐 Joplin `addNote` 的 hasNote 短路）。
    pub fn note_has_tag(&self, note_id: &str, tag_id: &str) -> bool {
        self.note_tags.iter().any(|nt| nt.note_id == note_id && nt.tag_id == tag_id)
    }

    /// (note,tag) 对应的全部 note_tag 条目 id（Joplin `removeNote` 删全部匹配）。
    pub fn note_tag_ids_for(&self, note_id: &str, tag_id: &str) -> Vec<String> {
        self.note_tags
            .iter()
            .filter(|nt| nt.note_id == note_id && nt.tag_id == tag_id)
            .map(|nt| nt.id.clone())
            .collect()
    }

    /// 某笔记的标签，按标题（不区分大小写）排序。
    pub fn tags_of_note(&self, note_id: &str) -> Vec<&Tag> {
        let mut ids: HashSet<&str> = HashSet::new();
        let mut out: Vec<&Tag> = Vec::new();
        for nt in &self.note_tags {
            if nt.note_id == note_id {
                if let Some(tag) = self.tags.get(&nt.tag_id) {
                    if ids.insert(tag.id.as_str()) {
                        out.push(tag);
                    }
                }
            }
        }
        out.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));
        out
    }

    pub fn note(&self, id: &str) -> Option<&Note> {
        self.notes.get(id)
    }

    pub fn resource(&self, id: &str) -> Option<&Resource> {
        self.resources.get(id)
    }

    /// 简单全文搜索：标题/正文不区分大小写包含。按更新时间倒序，限制 200 条。
    pub fn search(&self, query: &str) -> Vec<&Note> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return vec![];
        }
        let mut hits: Vec<&Note> = self
            .notes
            .values()
            .filter(|n| {
                n.title.to_lowercase().contains(&q) || n.body.to_lowercase().contains(&q)
            })
            .collect();
        hits.sort_by(|a, b| b.updated_time.cmp(&a.updated_time));
        hits.truncate(200);
        hits
    }

    /// 笔记的原始 .md 内容（保存时用于保留元数据）。
    pub fn note_raw(&self, id: &str) -> Option<&str> {
        self.raw_notes.get(id).map(|s| s.as_str())
    }

    /// 新增或更新一篇笔记（写回成功后同步内存）。返回笔记 id。
    pub fn upsert_note(&mut self, content: &str) -> anyhow::Result<String> {
        let raw = parser::parse_item(content)?;
        let note = parser::to_note(&raw)?;
        let id = note.id.clone();
        self.raw_notes.insert(id.clone(), content.to_string());
        self.notes.insert(id.clone(), note);
        self.build_indexes();
        Ok(id)
    }

    /// 新增或更新一个笔记本（写回成功后同步内存）。返回笔记本 id。
    pub fn upsert_folder(&mut self, content: &str) -> anyhow::Result<String> {
        let raw = parser::parse_item(content)?;
        let folder = parser::to_folder(&raw)?;
        let id = folder.id.clone();
        self.folders.insert(id.clone(), folder);
        self.build_indexes();
        Ok(id)
    }

    /// `candidate` 是否就是 `root` 本身、或位于 `root` 子树之下。
    /// 用于移动笔记本时防止把笔记本移进它自己或其后代（成环）。
    pub fn is_self_or_descendant(&self, root: &str, candidate: &str) -> bool {
        let mut cur = candidate.to_string();
        // 上限步数防御坏数据里的既有环，避免死循环
        for _ in 0..=self.folders.len() {
            if cur == root {
                return true;
            }
            match self.folders.get(&cur) {
                Some(f) if !f.parent_id.is_empty() => cur = f.parent_id.clone(),
                _ => return false,
            }
        }
        false
    }

    /// 某笔记本自身 + 其所有后代笔记本 id（BFS）。`root` 不存在则返回空。
    /// 用于访问控制的黑白名单按子树展开（server::auth::AuthState::scope）。
    pub fn subtree_folder_ids(&self, root: &str) -> Vec<String> {
        let mut out = Vec::new();
        if root.is_empty() || !self.folders.contains_key(root) {
            return out; // 空串=未分类根不是真笔记本，无子树
        }
        let mut stack = vec![root.to_string()];
        // 步数上限防坏数据成环
        let cap = self.folders.len() + 1;
        while let Some(id) = stack.pop() {
            if out.len() > cap {
                break;
            }
            if let Some(children) = self.child_folders.get(&id) {
                stack.extend(children.iter().cloned());
            }
            out.push(id);
        }
        out
    }

    /// 某笔记本子树内的全部笔记 id（自身 + 各层后代笔记本的直属笔记）。
    /// 用于级联删除笔记本：调用方据此逐条从存储删除再同步内存。
    /// `root` 为空串（未分类根）或不存在 → 空。
    pub fn subtree_note_ids(&self, root: &str) -> Vec<String> {
        self.subtree_folder_ids(root)
            .iter()
            .filter_map(|fid| self.notes_by_folder.get(fid))
            .flatten()
            .cloned()
            .collect()
    }

    /// 批量删除笔记本与笔记（写回成功后同步内存；索引只重建一次）。
    /// 级联删除笔记本子树用：调用方先据 [`Self::subtree_folder_ids`] /
    /// [`Self::subtree_note_ids`] 取 id，**存储删成功多少就传多少**，
    /// 保证内存索引与存储一致（部分失败时不至于凭空少掉还在盘上的条目）。
    pub fn remove_folders_and_notes(&mut self, folder_ids: &[String], note_ids: &[String]) {
        for id in note_ids {
            self.notes.remove(id);
            self.raw_notes.remove(id);
        }
        for id in folder_ids {
            self.folders.remove(id);
        }
        self.build_indexes();
    }

    /// 新增或更新一个资源（上传成功后同步内存，使 /api/resources 能拿到 mime）。返回资源 id。
    pub fn upsert_resource(&mut self, content: &str) -> anyhow::Result<String> {
        let raw = parser::parse_item(content)?;
        let r = parser::to_resource(&raw)?;
        let id = r.id.clone();
        self.resources.insert(id.clone(), r);
        Ok(id)
    }

    /// 删除一篇笔记（写回成功后同步内存）。
    pub fn remove_note(&mut self, id: &str) {
        self.notes.remove(id);
        self.raw_notes.remove(id);
        self.build_indexes();
    }

    /// 新增或更新一个标签（写回成功后同步内存）。返回标签 id。
    pub fn upsert_tag(&mut self, content: &str) -> anyhow::Result<String> {
        let raw = parser::parse_item(content)?;
        let tag = parser::to_tag(&raw)?;
        let id = tag.id.clone();
        self.tags.insert(id.clone(), tag);
        // 标签本身不入 notes_by_tag（成员由 note_tag 决定），无需重建索引。
        Ok(id)
    }

    /// 新增一个 note_tag 关联（写回成功后同步内存）。按 id 去重后重建标签成员索引。
    pub fn upsert_note_tag(&mut self, content: &str) -> anyhow::Result<NoteTag> {
        let raw = parser::parse_item(content)?;
        let nt = parser::to_note_tag(&raw)?;
        self.note_tags.retain(|x| x.id != nt.id);
        self.note_tags.push(nt.clone());
        self.build_indexes();
        Ok(nt)
    }

    /// 删除一个 note_tag 关联（按条目 id；写回成功后同步内存）。
    pub fn remove_note_tag(&mut self, id: &str) {
        self.note_tags.retain(|nt| nt.id != id);
        self.build_indexes();
    }

    /// 删除一个资源（写回成功后同步内存）。
    pub fn remove_resource(&mut self, id: &str) {
        self.resources.remove(id);
    }

    /// 每个资源被多少篇笔记引用（据 `resource_notes` 反向索引求长度）。
    /// 未出现在结果中的资源即为无人引用的“孤儿”。
    pub fn resource_usage(&self) -> HashMap<String, usize> {
        self.resource_notes.iter().map(|(id, notes)| (id.clone(), notes.len())).collect()
    }

    /// 引用某资源的笔记（用于访问控制的 resource→note→folder 权限链路：
    /// 资源本身不知道自己在哪个笔记本下，只能通过引用它的笔记归属间接判定可见性）。
    /// 无人引用（孤儿资源）→ 空 vec。
    pub fn notes_referencing_resource(&self, resource_id: &str) -> Vec<&Note> {
        self.resource_notes
            .get(resource_id)
            .map(|ids| ids.iter().filter_map(|id| self.notes.get(id)).collect())
            .unwrap_or_default()
    }
}

/// 统计 markdown 任务清单（GFM checkbox）的完成/总数：`(已完成, 总数)`。
/// 仅认行首（去缩进后）形如 `- [ ] ` / `* [x] ` / `+ [X] ` 的列表项。
pub fn count_tasks(body: &str) -> (usize, usize) {
    let mut done = 0;
    let mut total = 0;
    for line in body.lines() {
        let b = line.trim_start().as_bytes();
        // `<-|*|+>` SP `[` <mark> `]`  且其后是空白或行尾
        if b.len() >= 5
            && matches!(b[0], b'-' | b'*' | b'+')
            && b[1] == b' '
            && b[2] == b'['
            && b[4] == b']'
        {
            let after_ok = b.len() == 5 || b[5] == b' ' || b[5] == b'\t';
            match (after_ok, b[3]) {
                (true, b' ') => total += 1,
                (true, b'x') | (true, b'X') => {
                    total += 1;
                    done += 1;
                }
                _ => {}
            }
        }
    }
    (done, total)
}

/// 扫描文本里的 Joplin 资源引用 `:/<32hex>`，返回去重后的 id 集合（每篇笔记内同一资源只计一次）。
fn scan_resource_refs(body: &str) -> HashSet<String> {
    let b = body.as_bytes();
    let mut out = HashSet::new();
    let mut i = 0;
    while i + 34 <= b.len() {
        // `:/` 后接恰好 32 个十六进制，且第 33 位不再是十六进制（id 长度精确为 32）
        if b[i] == b':' && b[i + 1] == b'/' {
            let hex = &b[i + 2..i + 34];
            let bounded = i + 34 >= b.len() || !b[i + 34].is_ascii_hexdigit();
            if bounded && hex.iter().all(|c| c.is_ascii_hexdigit()) {
                out.insert(String::from_utf8_lossy(hex).to_lowercase());
                i += 34;
                continue;
            }
        }
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_resource_refs() {
        let body = "见图 ![a](:/0123456789abcdef0123456789abcdef) 和附件 [f](:/0123456789ABCDEF0123456789ABCDEF)\n\
            重复同一个 :/0123456789abcdef0123456789abcdef 再来个 :/deadbeefdeadbeefdeadbeefdeadbeef";
        let refs = scan_resource_refs(body);
        // 大小写归一后，第一个引用去重为 1 个，另有 deadbeef 一个 → 共 2 个不同 id
        assert_eq!(refs.len(), 2);
        assert!(refs.contains("0123456789abcdef0123456789abcdef"));
        assert!(refs.contains("deadbeefdeadbeefdeadbeefdeadbeef"));
    }

    #[test]
    fn counts_task_list() {
        let body = "标题\n- [ ] 待办一\n- [x] 已完成\n* [X] 也完成\n+ [ ] 第四\n普通行\n-[ ] 无空格不算\n- [] 非法不算";
        assert_eq!(count_tasks(body), (2, 4));
        assert_eq!(count_tasks("没有任何任务"), (0, 0));
        assert_eq!(count_tasks("  - [x] 缩进也算"), (1, 1));
    }

    #[test]
    fn ignores_non_32hex() {
        // 太短 / 太长 / 非 hex 都不算
        let refs = scan_resource_refs(":/short :/0123456789abcdef0123456789abcdefEXTRA :/zzzz");
        assert!(refs.is_empty());
    }

    // ---- 下面用 serialize 造 .md 喂 from_contents，做 Library 级集成单测 ----
    use crate::serialize::{new_folder_md, new_note_md, new_note_tag_md, new_resource_md, new_tag_md};

    fn hid(n: u8) -> String {
        // 造一个确定的 32hex id：把一个字节重复 16 次的十六进制
        format!("{:02x}", n).repeat(16)
    }

    #[test]
    fn builds_tree_counts_and_ordering() {
        let (root_a, root_b, child) = (hid(0xa0), hid(0xb0), hid(0xc0));
        let contents = vec![
            new_folder_md(&root_a, "", "Alpha", 1000),
            new_folder_md(&root_b, "", "Beta", 2000),
            new_folder_md(&child, &root_a, "Child", 1500),
            new_note_md(&hid(1), &root_a, "n1", "body one", false, 100),
            new_note_md(&hid(2), &root_a, "n2", "body two", false, 200),
            new_note_md(&hid(3), &child, "n3", "nested", false, 300),
        ];
        let (lib, stats) = Library::from_contents(contents);
        assert_eq!(stats.folders, 3);
        assert_eq!(stats.notes, 3);
        assert_eq!(stats.errors, 0);

        // 直属笔记数：root_a 有 2，child 有 1，root_b 有 0
        assert_eq!(lib.note_count(&root_a), 2);
        assert_eq!(lib.note_count(&child), 1);
        assert_eq!(lib.note_count(&root_b), 0);

        // 根下的子笔记本按标题排序：Alpha < Beta
        let roots = lib.child_folder_ids_sorted("");
        assert_eq!(roots, vec![root_a.clone(), root_b.clone()]);

        // 笔记按更新时间倒序：n2(200) 在 n1(100) 前
        let notes = lib.notes_in_folder_sorted(&root_a);
        assert_eq!(
            notes.iter().map(|n| n.title.as_str()).collect::<Vec<_>>(),
            vec!["n2", "n1"]
        );
    }

    #[test]
    fn search_matches_title_and_body_case_insensitively() {
        let contents = vec![
            new_note_md(&hid(1), "", "Rust Notes", "hello world", false, 100),
            new_note_md(&hid(2), "", "Cooking", "about RUST macros", false, 200),
            new_note_md(&hid(3), "", "Unrelated", "nothing here", false, 300),
        ];
        let (lib, _) = Library::from_contents(contents);

        // 命中标题(n1) + 正文(n2)，不区分大小写；按更新时间倒序 → n2 在前
        let hits = lib.search("rust");
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].title, "Cooking");
        assert_eq!(hits[1].title, "Rust Notes");

        assert!(lib.search("   ").is_empty()); // 空查询
        assert!(lib.search("zzz-no-match").is_empty());
    }

    #[test]
    fn anti_cycle_self_or_descendant() {
        // 链：A -> B -> C（C 在 A 子树下），D 是独立根
        let (a, b, c, d) = (hid(0xa0), hid(0xb0), hid(0xc0), hid(0xd0));
        let contents = vec![
            new_folder_md(&a, "", "A", 1),
            new_folder_md(&b, &a, "B", 2),
            new_folder_md(&c, &b, "C", 3),
            new_folder_md(&d, "", "D", 4),
        ];
        let (lib, _) = Library::from_contents(contents);

        assert!(lib.is_self_or_descendant(&a, &a)); // 自身
        assert!(lib.is_self_or_descendant(&a, &c)); // 后代（禁止把 A 移到 C 下）
        assert!(lib.is_self_or_descendant(&b, &c)); // 直接子
        assert!(!lib.is_self_or_descendant(&c, &a)); // 祖先不是后代 → 允许
        assert!(!lib.is_self_or_descendant(&a, &d)); // 无关分支
    }

    /// 级联删除笔记本子树：subtree_note_ids 收全各层笔记，
    /// remove_folders_and_notes 删完后索引（树/篇数/搜索）同步干净，无关分支不受影响。
    #[test]
    fn subtree_notes_and_cascade_removal() {
        // A -> B -> C（各一篇笔记），D 是独立根（一篇笔记）
        let (a, b, c, d) = (hid(0xa0), hid(0xb0), hid(0xc0), hid(0xd0));
        let (n_a, n_b, n_c, n_d) = (hid(1), hid(2), hid(3), hid(4));
        let contents = vec![
            new_folder_md(&a, "", "A", 1),
            new_folder_md(&b, &a, "B", 2),
            new_folder_md(&c, &b, "C", 3),
            new_folder_md(&d, "", "D", 4),
            new_note_md(&n_a, &a, "na", "body a", false, 100),
            new_note_md(&n_b, &b, "nb", "body b", false, 200),
            new_note_md(&n_c, &c, "nc", "body c", false, 300),
            new_note_md(&n_d, &d, "nd", "body d", false, 400),
        ];
        let (mut lib, _) = Library::from_contents(contents);

        // B 子树 = B + C，笔记 = nb + nc（不含祖先 A 的 na，也不含无关的 nd）
        let mut folders = lib.subtree_folder_ids(&b);
        folders.sort();
        let mut expect_folders = vec![b.clone(), c.clone()];
        expect_folders.sort();
        assert_eq!(folders, expect_folders);
        let mut notes = lib.subtree_note_ids(&b);
        notes.sort();
        let mut expect_notes = vec![n_b.clone(), n_c.clone()];
        expect_notes.sort();
        assert_eq!(notes, expect_notes);

        // 未分类根 / 不存在的 id 无子树
        assert!(lib.subtree_note_ids("").is_empty());
        assert!(lib.subtree_note_ids(&hid(0xee)).is_empty());

        lib.remove_folders_and_notes(&folders, &notes);

        assert!(!lib.folders.contains_key(&b) && !lib.folders.contains_key(&c));
        assert!(!lib.notes.contains_key(&n_b) && !lib.notes.contains_key(&n_c));
        assert!(lib.note_raw(&n_b).is_none()); // 原始内容一并释放
        assert!(lib.child_folder_ids_sorted(&a).is_empty()); // 树索引已重建
        assert!(lib.search("body b").is_empty());
        // 无关分支完好
        assert_eq!(lib.note_count(&a), 1);
        assert_eq!(lib.note_count(&d), 1);
        assert_eq!(lib.child_folder_ids_sorted(""), vec![a.clone(), d.clone()]);
    }

    // 造一个 tag（type_=5，带标题）/ note_tag（type_=6，纯元数据）条目内容。
    fn tag_md(id: &str, title: &str) -> String {
        format!("{title}\n\nid: {id}\nparent_id: \ntype_: 5")
    }
    fn note_tag_md(id: &str, note_id: &str, tag_id: &str) -> String {
        format!("id: {id}\nnote_id: {note_id}\ntag_id: {tag_id}\ntype_: 6")
    }

    #[test]
    fn indexes_tags_with_counts_and_membership() {
        let (t_work, t_idea) = (hid(0x51), hid(0x52));
        let (n1, n2, n3) = (hid(1), hid(2), hid(3));
        let contents = vec![
            new_note_md(&n1, "", "n1", "body", false, 100),
            new_note_md(&n2, "", "n2", "body", false, 300),
            new_note_md(&n3, "", "n3", "body", false, 200),
            tag_md(&t_work, "Work"),
            tag_md(&t_idea, "idea"),
            // n1 与 n2 打了 Work；n3 打了 idea；重复关联应被去重
            note_tag_md(&hid(0x61), &n1, &t_work),
            note_tag_md(&hid(0x62), &n2, &t_work),
            note_tag_md(&hid(0x63), &n2, &t_work), // 重复：不重复计数
            note_tag_md(&hid(0x64), &n3, &t_idea),
            // 悬挂关联：引用不存在的笔记，应被忽略
            note_tag_md(&hid(0x65), &hid(0xff), &t_work),
        ];
        let (lib, stats) = Library::from_contents(contents);
        assert_eq!(stats.tags, 2);
        assert_eq!(stats.note_tags, 5);

        // 标签按标题不区分大小写排序：idea < Work
        let tags = lib.tags_sorted();
        assert_eq!(tags.iter().map(|t| t.title.as_str()).collect::<Vec<_>>(), vec!["idea", "Work"]);

        // 计数：Work 去重后 2（n1,n2），idea 1（n3）
        assert_eq!(lib.tag_note_count(&t_work), 2);
        assert_eq!(lib.tag_note_count(&t_idea), 1);
        assert_eq!(lib.tag_note_count(&hid(0xee)), 0); // 未知标签

        // 成员：Work 下按更新时间倒序 → n2(300) 在 n1(100) 前
        let work_notes = lib.notes_with_tag(&t_work);
        assert_eq!(work_notes.iter().map(|n| n.title.as_str()).collect::<Vec<_>>(), vec!["n2", "n1"]);
        assert!(lib.notes_with_tag(&hid(0xee)).is_empty());
    }

    #[test]
    fn tag_mutations_add_reuse_and_remove() {
        let (n1, n2) = (hid(1), hid(2));
        let (mut lib, _) = Library::from_contents(vec![
            new_note_md(&n1, "", "n1", "b", false, 100),
            new_note_md(&n2, "", "n2", "b", false, 200),
        ]);

        // 新建标签 + 关联 n1
        let tag_id = hid(0x51);
        lib.upsert_tag(&new_tag_md(&tag_id, "Work", 1)).unwrap();
        assert_eq!(lib.tag_id_by_title("work").as_deref(), Some(tag_id.as_str())); // 不区分大小写
        assert_eq!(lib.tag_id_by_title("  WORK  ").as_deref(), Some(tag_id.as_str())); // trim
        assert_eq!(lib.tag_id_by_title("nope"), None);

        assert!(!lib.note_has_tag(&n1, &tag_id));
        let nt = lib.upsert_note_tag(&new_note_tag_md(&hid(0x61), &n1, &tag_id, 1)).unwrap();
        assert_eq!(nt.note_id, n1);
        assert!(lib.note_has_tag(&n1, &tag_id));
        assert_eq!(lib.tag_note_count(&tag_id), 1);
        assert_eq!(lib.notes_with_tag(&tag_id).iter().map(|n| n.title.as_str()).collect::<Vec<_>>(), vec!["n1"]);
        assert_eq!(lib.tags_of_note(&n1).iter().map(|t| t.title.as_str()).collect::<Vec<_>>(), vec!["Work"]);
        assert!(lib.tags_of_note(&n2).is_empty());

        // 再关联 n2；成员两条
        lib.upsert_note_tag(&new_note_tag_md(&hid(0x62), &n2, &tag_id, 1)).unwrap();
        assert_eq!(lib.tag_note_count(&tag_id), 2);

        // 移除 n1 的关联：按 (note,tag) 查 id 再删
        let ids = lib.note_tag_ids_for(&n1, &tag_id);
        assert_eq!(ids, vec![hid(0x61)]);
        lib.remove_note_tag(&ids[0]);
        assert!(!lib.note_has_tag(&n1, &tag_id));
        assert_eq!(lib.tag_note_count(&tag_id), 1);
        assert_eq!(lib.notes_with_tag(&tag_id).iter().map(|n| n.title.as_str()).collect::<Vec<_>>(), vec!["n2"]);

        // upsert 同 id 关联幂等（不重复计数）
        lib.upsert_note_tag(&new_note_tag_md(&hid(0x62), &n2, &tag_id, 1)).unwrap();
        assert_eq!(lib.tag_note_count(&tag_id), 1);
    }

    #[test]
    fn tag_membership_drops_deleted_notes() {
        let t = hid(0x51);
        let (n1, n2) = (hid(1), hid(2));
        let contents = vec![
            new_note_md(&n1, "", "n1", "body", false, 100),
            new_note_md(&n2, "", "n2", "body", false, 200),
            tag_md(&t, "Work"),
            note_tag_md(&hid(0x61), &n1, &t),
            note_tag_md(&hid(0x62), &n2, &t),
        ];
        let (mut lib, _) = Library::from_contents(contents);
        assert_eq!(lib.tag_note_count(&t), 2);

        // 删除 n1 后，标签成员随索引重建而剔除（note_tag 悬挂无害）
        lib.remove_note(&n1);
        assert_eq!(lib.tag_note_count(&t), 1);
        assert_eq!(lib.notes_with_tag(&t).iter().map(|n| n.title.as_str()).collect::<Vec<_>>(), vec!["n2"]);
    }

    #[test]
    fn resource_usage_counts_refs_and_orphans() {
        let (r_used, r_orphan) = (hid(0xe0), hid(0xf0));
        let contents = vec![
            new_note_md(&hid(1), "", "n1", &format!("![x](:/{r_used})"), false, 1),
            new_note_md(&hid(2), "", "n2", &format!("again :/{r_used}"), false, 2),
            new_note_md(&hid(3), "", "n3", "no images here", false, 3),
            new_resource_md(&r_used, "used.png", "image/png", "png", 10, 1),
            new_resource_md(&r_orphan, "orphan.png", "image/png", "png", 10, 1),
        ];
        let (lib, stats) = Library::from_contents(contents);
        assert_eq!(stats.resources, 2);

        let usage = lib.resource_usage();
        assert_eq!(usage.get(&r_used).copied(), Some(2)); // 被 2 篇引用
        assert_eq!(usage.get(&r_orphan).copied(), None); // 孤儿：不出现
        assert!(lib.resource(&r_used).is_some());
        assert_eq!(lib.resource(&r_used).unwrap().mime, "image/png");
    }

    /// `notes_referencing_resource`：跨文件夹的多篇引用都能找到；孤儿资源返回空。
    /// 用于访问控制的 resource→note→folder 权限链路（server::api::resource）。
    #[test]
    fn notes_referencing_resource_finds_cross_folder_refs_and_empty_for_orphan() {
        let folder_a = hid(0xa0);
        let folder_b = hid(0xb0);
        let (r_used, r_orphan) = (hid(0xe0), hid(0xf0));
        let contents = vec![
            format!("A\n\nid: {folder_a}\nparent_id: \ntype_: 2"),
            format!("B\n\nid: {folder_b}\nparent_id: \ntype_: 2"),
            new_note_md(&hid(1), &folder_a, "n1", &format!("![x](:/{r_used})"), false, 1),
            new_note_md(&hid(2), &folder_b, "n2", &format!("again :/{r_used}"), false, 2),
            new_resource_md(&r_used, "used.png", "image/png", "png", 10, 1),
            new_resource_md(&r_orphan, "orphan.png", "image/png", "png", 10, 1),
        ];
        let (lib, _) = Library::from_contents(contents);

        let mut parents: Vec<String> =
            lib.notes_referencing_resource(&r_used).iter().map(|n| n.parent_id.clone()).collect();
        parents.sort();
        assert_eq!(parents, vec![folder_a, folder_b]);

        assert!(lib.notes_referencing_resource(&r_orphan).is_empty());
        assert!(lib.notes_referencing_resource(&hid(0xff)).is_empty()); // 未知 id
    }
}
