//! 浏览器 WASM 内核：把 jasper-core 编进浏览器，暴露与后端 /api 同形的读写查询
//! （返回 JSON 字符串，前端 JSON.parse）。
//!
//! 两种用法：
//! - **只读展示**（`Demo::new`，内置演示库）：GitHub Pages 上的零服务器预览。
//! - **可写本地应用**（`Demo::from_raws`，从 IndexedDB 恢复的原始条目）：浏览器里持久读写。
//!
//! 写方法**把 `now` 当参数从 JS 注入**（`Date.now()`）——`serialize::now_ms()` 用
//! `SystemTime::now()`，在 wasm32-unknown-unknown 上会 panic，故本层任何路径都不得调用它。
//! `new_id()` 走 getrandom 的 js 后端（浏览器 crypto），可用。

use jasper_core::library::{count_tasks, Library};
use jasper_core::model::{MarkupLanguage, Note, Resource, Tag};
use jasper_core::{parser, serialize};
use serde::Serialize;
use std::collections::HashMap;
use wasm_bindgen::prelude::*;

mod demo;

// ---- DTO：与 web/src/lib/api.ts 的接口一一对应 ----

#[derive(Serialize)]
struct FolderNode {
    id: String,
    title: String,
    note_count: usize,
    children: Vec<FolderNode>,
}

#[derive(Serialize)]
struct FolderRef {
    id: String,
    title: String,
    parent_id: String,
}

#[derive(Serialize)]
struct NoteSummary {
    id: String,
    title: String,
    updated_time: i64,
    parent_id: String,
    is_todo: bool,
    todo_completed: bool,
    task_done: usize,
    task_total: usize,
}

#[derive(Serialize)]
struct NoteDetail {
    id: String,
    title: String,
    body: String,
    markup_language: i32,
    parent_id: String,
    created_time: i64,
    updated_time: i64,
    source_url: String,
    is_todo: bool,
    todo_completed: bool,
}

#[derive(Serialize)]
struct TagInfo {
    id: String,
    title: String,
    note_count: usize,
}

#[derive(Serialize)]
struct TagRef {
    id: String,
    title: String,
}

#[derive(Serialize)]
struct ResourceInfo {
    id: String,
    title: String,
    mime: String,
    file_extension: String,
    size: i64,
    updated_time: i64,
    used_by: usize,
}

#[derive(Serialize)]
struct ResourceUpload {
    id: String,
    title: String,
    mime: String,
    file_extension: String,
    size: i64,
    markdown: String,
}

fn summarize(n: &Note) -> NoteSummary {
    let (task_done, task_total) = count_tasks(&n.body);
    NoteSummary {
        id: n.id.clone(),
        title: n.title.clone(),
        updated_time: n.updated_time,
        parent_id: n.parent_id.clone(),
        is_todo: n.is_todo,
        todo_completed: n.todo_completed,
        task_done,
        task_total,
    }
}

fn detail_of(n: &Note) -> NoteDetail {
    NoteDetail {
        id: n.id.clone(),
        title: n.title.clone(),
        body: n.body.clone(),
        markup_language: match n.markup_language {
            MarkupLanguage::Markdown => 1,
            MarkupLanguage::Html => 2,
        },
        parent_id: n.parent_id.clone(),
        created_time: n.created_time,
        updated_time: n.updated_time,
        source_url: n.source_url.clone(),
        is_todo: n.is_todo,
        todo_completed: n.todo_completed,
    }
}

fn tag_ref(t: &Tag) -> TagRef {
    TagRef { id: t.id.clone(), title: t.title.clone() }
}

fn resource_info(r: &Resource, usage: &HashMap<String, usize>) -> ResourceInfo {
    ResourceInfo {
        id: r.id.clone(),
        title: r.title.clone(),
        mime: r.mime.clone(),
        file_extension: r.file_extension.clone(),
        size: r.size,
        updated_time: r.updated_time,
        used_by: usage.get(&r.id).copied().unwrap_or(0),
    }
}

/// 从一条原始 `.md` 里取出条目 id（用于按 id 维护 raws 表）。
fn id_of(raw: &str) -> Option<String> {
    parser::parse_item(raw).ok().and_then(|r| r.props.get("id").cloned())
}

fn to_json<T: Serialize>(v: &T) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| "null".into())
}

fn now_i64(now: f64) -> i64 {
    now as i64
}

/// 浏览器侧的“库”：用一批原始条目构建，常驻内存。
///
/// 除 core 的 `Library`（解析后的索引）外，另存一份 **全类型** id→raw 表 `raws`：
/// - `Library` 只保留 notes 的原始内容，而 rename/move 笔记本、标签、资源需要各自的原始 `.md`
///   来逐字保留元数据；
/// - `raws` 同时是持久化快照的来源（`snapshot()`）。
#[wasm_bindgen]
pub struct Demo {
    lib: Library,
    raws: HashMap<String, String>,
}

impl Demo {
    fn build(raws: Vec<String>) -> Demo {
        let (lib, _stats) = Library::from_contents(raws.clone());
        let map = raws.into_iter().filter_map(|r| id_of(&r).map(|id| (id, r))).collect();
        Demo { lib, raws: map }
    }

    fn tree(&self, parent: &str) -> Vec<FolderNode> {
        self.lib
            .child_folder_ids_sorted(parent)
            .into_iter()
            .filter_map(|id| {
                self.lib.folders.get(&id).map(|f| FolderNode {
                    id: f.id.clone(),
                    title: f.title.clone(),
                    note_count: self.lib.note_count(&f.id),
                    children: self.tree(&f.id),
                })
            })
            .collect()
    }

    /// 生成新条目、同步内存索引与 raws 表。返回新条目 id。
    fn commit(&mut self, content: String, kind: ItemKind) -> Result<String, JsError> {
        let id = match kind {
            ItemKind::Note => self.lib.upsert_note(&content),
            ItemKind::Folder => self.lib.upsert_folder(&content),
            ItemKind::Resource => self.lib.upsert_resource(&content),
            ItemKind::Tag => self.lib.upsert_tag(&content),
        }
        .map_err(|e| JsError::new(&e.to_string()))?;
        self.raws.insert(id.clone(), content);
        Ok(id)
    }
}

enum ItemKind {
    Note,
    Folder,
    Resource,
    Tag,
}

#[wasm_bindgen]
impl Demo {
    /// 只读展示：内置演示库。
    #[wasm_bindgen(constructor)]
    pub fn new() -> Demo {
        Demo::build(demo::items())
    }

    /// 可写本地应用：从持久化的原始条目恢复（空数组 = 空库）。
    #[wasm_bindgen(js_name = fromRaws)]
    pub fn from_raws(raws: Vec<String>) -> Demo {
        Demo::build(raws)
    }

    /// 首次运行的播种数据（内置演示库的原始条目），供前端写入 IndexedDB。
    #[wasm_bindgen(js_name = seedItems)]
    pub fn seed_items() -> Vec<String> {
        demo::items()
    }

    /// 全部原始条目（`.md`），供前端整体持久化。
    pub fn snapshot(&self) -> Vec<String> {
        self.raws.values().cloned().collect()
    }

    // ---------- 只读 ----------

    /// 笔记本树（嵌套 + 篇数），JSON。
    pub fn folders(&self) -> String {
        to_json(&self.tree(""))
    }

    /// 某笔记本下的笔记列表（按更新时间倒序），JSON。
    pub fn notes(&self, folder: &str) -> String {
        let list: Vec<NoteSummary> =
            self.lib.notes_in_folder_sorted(folder).into_iter().map(summarize).collect();
        to_json(&list)
    }

    /// 单篇笔记详情，JSON；不存在返回 "null"。
    pub fn note(&self, id: &str) -> String {
        match self.lib.note(id) {
            Some(n) => to_json(&detail_of(n)),
            None => "null".into(),
        }
    }

    /// 标题/正文全文搜索，JSON。
    pub fn search(&self, q: &str) -> String {
        let list: Vec<NoteSummary> = self.lib.search(q).into_iter().map(summarize).collect();
        to_json(&list)
    }

    /// 标签列表（含篇数），JSON。
    pub fn tags(&self) -> String {
        let list: Vec<TagInfo> = self
            .lib
            .tags_sorted()
            .into_iter()
            .map(|t| TagInfo {
                id: t.id.clone(),
                title: t.title.clone(),
                note_count: self.lib.tag_note_count(&t.id),
            })
            .collect();
        to_json(&list)
    }

    /// 打了某标签的笔记（按更新时间倒序），JSON。
    #[wasm_bindgen(js_name = notesByTag)]
    pub fn notes_by_tag(&self, tag_id: &str) -> String {
        let list: Vec<NoteSummary> =
            self.lib.notes_with_tag(tag_id).into_iter().map(summarize).collect();
        to_json(&list)
    }

    /// 某笔记的标签（按标题排序），JSON。
    #[wasm_bindgen(js_name = noteTags)]
    pub fn note_tags(&self, note_id: &str) -> String {
        let list: Vec<TagRef> = self.lib.tags_of_note(note_id).into_iter().map(tag_ref).collect();
        to_json(&list)
    }

    /// 资源清单（含 used_by 引用计数，孤儿在前、同级按体积降序），JSON。
    pub fn resources(&self) -> String {
        let usage = self.lib.resource_usage();
        let mut out: Vec<ResourceInfo> =
            self.lib.resources.values().map(|r| resource_info(r, &usage)).collect();
        out.sort_by(|a, b| a.used_by.cmp(&b.used_by).then(b.size.cmp(&a.size)));
        to_json(&out)
    }

    // ---------- 写（now:f64 由 JS 注入 Date.now()）----------

    /// 新建笔记，返回 NoteDetail JSON。
    #[wasm_bindgen(js_name = createNote)]
    pub fn create_note(
        &mut self,
        parent_id: &str,
        title: &str,
        body: &str,
        is_todo: bool,
        now: f64,
    ) -> Result<String, JsError> {
        let id = serialize::new_id();
        let content =
            serialize::new_note_md(&id, parent_id, title, body, is_todo, now_i64(now));
        let id = self.commit(content, ItemKind::Note)?;
        Ok(self.note(&id))
    }

    /// 更新笔记标题/正文，返回 NoteDetail JSON。
    #[wasm_bindgen(js_name = updateNote)]
    pub fn update_note(
        &mut self,
        id: &str,
        title: &str,
        body: &str,
        now: f64,
    ) -> Result<String, JsError> {
        let original =
            self.raws.get(id).cloned().ok_or_else(|| JsError::new("note not found"))?;
        let content = serialize::update_note_md(&original, title, body, now_i64(now))
            .map_err(|e| JsError::new(&e.to_string()))?;
        let id = self.commit(content, ItemKind::Note)?;
        Ok(self.note(&id))
    }

    /// 移动笔记到另一笔记本，返回 NoteDetail JSON。
    #[wasm_bindgen(js_name = moveNote)]
    pub fn move_note(&mut self, id: &str, new_parent: &str, now: f64) -> Result<String, JsError> {
        if !new_parent.is_empty() && !self.lib.folders.contains_key(new_parent) {
            return Err(JsError::new("target folder not found"));
        }
        let original =
            self.raws.get(id).cloned().ok_or_else(|| JsError::new("note not found"))?;
        let content = serialize::move_note_md(&original, new_parent, now_i64(now))
            .map_err(|e| JsError::new(&e.to_string()))?;
        let id = self.commit(content, ItemKind::Note)?;
        Ok(self.note(&id))
    }

    /// 删除笔记。
    #[wasm_bindgen(js_name = deleteNote)]
    pub fn delete_note(&mut self, id: &str) {
        self.lib.remove_note(id);
        self.raws.remove(id);
    }

    /// 新建笔记本，返回 FolderRef JSON。
    #[wasm_bindgen(js_name = createFolder)]
    pub fn create_folder(&mut self, parent_id: &str, title: &str, now: f64) -> Result<String, JsError> {
        if !parent_id.is_empty() && !self.lib.folders.contains_key(parent_id) {
            return Err(JsError::new("parent folder not found"));
        }
        let id = serialize::new_id();
        let content = serialize::new_folder_md(&id, parent_id, title, now_i64(now));
        let id = self.commit(content, ItemKind::Folder)?;
        Ok(self.folder_ref_json(&id))
    }

    /// 重命名笔记本，返回 FolderRef JSON。
    #[wasm_bindgen(js_name = renameFolder)]
    pub fn rename_folder(&mut self, id: &str, title: &str, now: f64) -> Result<String, JsError> {
        let original =
            self.raws.get(id).cloned().ok_or_else(|| JsError::new("folder not found"))?;
        let content = serialize::rename_folder_md(&original, title, now_i64(now))
            .map_err(|e| JsError::new(&e.to_string()))?;
        let id = self.commit(content, ItemKind::Folder)?;
        Ok(self.folder_ref_json(&id))
    }

    /// 移动笔记本（防环：禁移进自身或后代），返回 FolderRef JSON。
    #[wasm_bindgen(js_name = moveFolder)]
    pub fn move_folder(&mut self, id: &str, new_parent: &str, now: f64) -> Result<String, JsError> {
        if !new_parent.is_empty() && !self.lib.folders.contains_key(new_parent) {
            return Err(JsError::new("target folder not found"));
        }
        if !new_parent.is_empty() && self.lib.is_self_or_descendant(id, new_parent) {
            return Err(JsError::new("cannot move a folder into itself or its descendant"));
        }
        let original =
            self.raws.get(id).cloned().ok_or_else(|| JsError::new("folder not found"))?;
        let content = serialize::move_folder_md(&original, new_parent, now_i64(now))
            .map_err(|e| JsError::new(&e.to_string()))?;
        let id = self.commit(content, ItemKind::Folder)?;
        Ok(self.folder_ref_json(&id))
    }

    /// 删除笔记本，级联删除其全部后代笔记本与其中的笔记（与 server 的
    /// `DELETE /api/folders/{id}` 同语义）。「未分类」(空 id) 不是真笔记本，报错。
    #[wasm_bindgen(js_name = deleteFolder)]
    pub fn delete_folder(&mut self, id: &str) -> Result<(), JsError> {
        if !self.lib.folders.contains_key(id) {
            return Err(JsError::new("folder not found"));
        }
        let folders = self.lib.subtree_folder_ids(id);
        let notes = self.lib.subtree_note_ids(id);
        for key in folders.iter().chain(notes.iter()) {
            self.raws.remove(key);
        }
        self.lib.remove_folders_and_notes(&folders, &notes);
        Ok(())
    }

    /// 给笔记打标签（trim + 不区分大小写复用/新建），返回该笔记的 TagRef[] JSON。
    #[wasm_bindgen(js_name = addNoteTag)]
    pub fn add_note_tag(&mut self, note_id: &str, title: &str, now: f64) -> Result<String, JsError> {
        let title = title.trim().to_string();
        if title.is_empty() {
            return Err(JsError::new("empty tag title"));
        }
        if self.lib.note(note_id).is_none() {
            return Err(JsError::new("note not found"));
        }
        let now = now_i64(now);
        match self.lib.tag_id_by_title(&title) {
            // 已有该标签且已关联 → 幂等
            Some(tid) if self.lib.note_has_tag(note_id, &tid) => {}
            Some(tid) => {
                let ntid = serialize::new_id();
                let content = serialize::new_note_tag_md(&ntid, note_id, &tid, now);
                self.lib
                    .upsert_note_tag(&content)
                    .map_err(|e| JsError::new(&e.to_string()))?;
                self.raws.insert(ntid, content);
            }
            None => {
                let tid = serialize::new_id();
                let tag_content = serialize::new_tag_md(&tid, &title, now);
                self.commit(tag_content, ItemKind::Tag)?;
                let ntid = serialize::new_id();
                let nt_content = serialize::new_note_tag_md(&ntid, note_id, &tid, now);
                self.lib
                    .upsert_note_tag(&nt_content)
                    .map_err(|e| JsError::new(&e.to_string()))?;
                self.raws.insert(ntid, nt_content);
            }
        }
        Ok(self.note_tags(note_id))
    }

    /// 从笔记去掉某标签（删关联，保留标签），返回该笔记的 TagRef[] JSON。
    #[wasm_bindgen(js_name = removeNoteTag)]
    pub fn remove_note_tag(&mut self, note_id: &str, tag_id: &str) -> String {
        for ntid in self.lib.note_tag_ids_for(note_id, tag_id) {
            self.lib.remove_note_tag(&ntid);
            self.raws.remove(&ntid);
        }
        self.note_tags(note_id)
    }

    /// 登记资源元数据（二进制由前端存 IndexedDB）。返回 ResourceUpload JSON。
    #[wasm_bindgen(js_name = upsertResourceMeta)]
    pub fn upsert_resource_meta(
        &mut self,
        id: &str,
        title: &str,
        mime: &str,
        file_extension: &str,
        size: f64,
        now: f64,
    ) -> Result<String, JsError> {
        let content = serialize::new_resource_md(
            id,
            title,
            mime,
            file_extension,
            size as i64,
            now_i64(now),
        );
        self.commit(content, ItemKind::Resource)?;
        let markdown = if mime.starts_with("image/") {
            format!("![{title}](:/{id})")
        } else {
            format!("[{title}](:/{id})")
        };
        Ok(to_json(&ResourceUpload {
            id: id.to_string(),
            title: title.to_string(),
            mime: mime.to_string(),
            file_extension: file_extension.to_string(),
            size: size as i64,
            markdown,
        }))
    }

    /// 重命名资源，返回 ResourceInfo JSON。
    #[wasm_bindgen(js_name = renameResource)]
    pub fn rename_resource(&mut self, id: &str, title: &str, now: f64) -> Result<String, JsError> {
        let original =
            self.raws.get(id).cloned().ok_or_else(|| JsError::new("resource not found"))?;
        let content = serialize::update_resource_md(&original, title.trim(), now_i64(now))
            .map_err(|e| JsError::new(&e.to_string()))?;
        self.commit(content, ItemKind::Resource)?;
        let usage = self.lib.resource_usage();
        let r = self.lib.resource(id).ok_or_else(|| JsError::new("resource not found"))?;
        Ok(to_json(&resource_info(r, &usage)))
    }

    /// 删除资源（元数据；二进制由前端从 IndexedDB 删）。
    #[wasm_bindgen(js_name = deleteResource)]
    pub fn delete_resource(&mut self, id: &str) {
        self.lib.remove_resource(id);
        self.raws.remove(id);
    }
}

impl Demo {
    fn folder_ref_json(&self, id: &str) -> String {
        match self.lib.folders.get(id) {
            Some(f) => to_json(&FolderRef {
                id: f.id.clone(),
                title: f.title.clone(),
                parent_id: f.parent_id.clone(),
            }),
            None => "null".into(),
        }
    }
}
