//! Joplin 条目文件（`<id>.md`）解析器。
//!
//! 算法逆向自 joplin/packages/lib/models/BaseItem.ts:589-632 (unserialize)
//! 并已对 JopinData 真实数据验证（笔记/笔记本/资源/标签/note_tag）。
//!
//! 文件结构（段间以单个空行分隔）：
//!   <标题>\n\n<正文 markdown>\n\n<key: value 元数据块>
//! 笔记(type_=1)才有正文段；note_tag(type_=6) 无标题无空行（纯元数据）。

use crate::model::*;
use anyhow::{anyhow, Result};
use std::collections::HashMap;

/// 解析一个条目文件内容为 RawItem。
pub fn parse_item(content: &str) -> Result<RawItem> {
    let lines: Vec<&str> = content.split('\n').collect();
    let mut props: HashMap<String, String> = HashMap::new();

    // 从文件末尾向上扫描，连续的非空行=元数据块，遇到第一个空行即为分隔符。
    // 若全程没有空行（如 note_tag），则整篇都是元数据，正文段为空。
    let mut body_section_len = 0usize;
    let mut found_separator = false;
    let mut i = lines.len();
    while i > 0 {
        i -= 1;
        let line = lines[i].trim();
        if line.is_empty() {
            body_section_len = i; // lines[0..i] 是 标题+正文 段
            found_separator = true;
            break;
        }
        let p = line
            .find(':')
            .ok_or_else(|| anyhow!("Invalid property line: {line:?}"))?;
        let key = line[..p].trim().to_string();
        let value = unescape(line[p + 1..].trim());
        // 自底向上，先出现的（更靠下的）保留；正常不会有重复键。
        props.entry(key).or_insert(value);
    }
    if !found_separator {
        body_section_len = 0;
    }

    let type_ = props
        .get("type_")
        .ok_or_else(|| anyhow!("Missing required property: type_"))?
        .parse::<i64>()
        .map_err(|_| anyhow!("Invalid type_"))?;

    let body_section = &lines[..body_section_len];
    let title = body_section.first().map(|s| s.to_string());
    // 正文 = 跳过标题(0)与其后的空行(1)，其余 join。仅笔记取正文。
    let body = if type_ == ItemType::Note as i64 && body_section.len() >= 2 {
        Some(body_section[2..].join("\n"))
    } else {
        None
    };

    Ok(RawItem {
        type_,
        title,
        body,
        props,
    })
}

/// 反转义元数据字段值。
/// 来源：BaseItem.ts:444-449 unserialize_format。
/// 顺序需与源码一致：\\n→\n、\\r→\r、\\\n→\\n、\\\r→\\r。
fn unescape(s: &str) -> String {
    if !s.contains('\\') {
        return s.to_string();
    }
    s.replace("\\n", "\n")
        .replace("\\r", "\r")
        .replace("\\\n", "\\n")
        .replace("\\\r", "\\r")
}

/// 解析 ISO `YYYY-MM-DDTHH:mm:ss.SSSZ`（UTC）为 Unix 毫秒。空值=0。
/// 来源：BaseItem.ts:436-438。
pub fn parse_time_ms(s: Option<&str>) -> i64 {
    let s = match s {
        Some(s) if !s.is_empty() => s,
        _ => return 0,
    };
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.timestamp_millis())
        .unwrap_or(0)
}

fn s(raw: &RawItem, key: &str) -> String {
    raw.prop(key).unwrap_or("").to_string()
}

fn i(raw: &RawItem, key: &str) -> i64 {
    raw.prop(key).and_then(|v| v.parse::<i64>().ok()).unwrap_or(0)
}

fn b(raw: &RawItem, key: &str) -> bool {
    raw.prop(key).map(|v| v == "1").unwrap_or(false)
}

pub fn to_note(raw: &RawItem) -> Result<Note> {
    Ok(Note {
        id: raw.id().ok_or_else(|| anyhow!("note missing id"))?.to_string(),
        parent_id: s(raw, "parent_id"),
        title: raw.title.clone().unwrap_or_default(),
        body: raw.body.clone().unwrap_or_default(),
        created_time: parse_time_ms(raw.prop("created_time")),
        updated_time: parse_time_ms(raw.prop("updated_time")),
        markup_language: MarkupLanguage::from_i64(i(raw, "markup_language")),
        is_todo: b(raw, "is_todo"),
        todo_completed: i(raw, "todo_completed") != 0,
        is_conflict: b(raw, "is_conflict"),
        source_url: s(raw, "source_url"),
        order: i(raw, "order"),
    })
}

pub fn to_folder(raw: &RawItem) -> Result<Folder> {
    Ok(Folder {
        id: raw.id().ok_or_else(|| anyhow!("folder missing id"))?.to_string(),
        parent_id: s(raw, "parent_id"),
        title: raw.title.clone().unwrap_or_default(),
        created_time: parse_time_ms(raw.prop("created_time")),
        updated_time: parse_time_ms(raw.prop("updated_time")),
        icon: s(raw, "icon"),
    })
}

pub fn to_resource(raw: &RawItem) -> Result<Resource> {
    Ok(Resource {
        id: raw.id().ok_or_else(|| anyhow!("resource missing id"))?.to_string(),
        title: raw.title.clone().unwrap_or_default(),
        mime: s(raw, "mime"),
        file_extension: s(raw, "file_extension"),
        size: i(raw, "size"),
        updated_time: parse_time_ms(raw.prop("updated_time")),
    })
}

pub fn to_tag(raw: &RawItem) -> Result<Tag> {
    Ok(Tag {
        id: raw.id().ok_or_else(|| anyhow!("tag missing id"))?.to_string(),
        parent_id: s(raw, "parent_id"),
        title: raw.title.clone().unwrap_or_default(),
    })
}

pub fn to_note_tag(raw: &RawItem) -> Result<NoteTag> {
    Ok(NoteTag {
        id: raw.id().ok_or_else(|| anyhow!("note_tag missing id"))?.to_string(),
        note_id: s(raw, "note_id"),
        tag_id: s(raw, "tag_id"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn data_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../JopinData")
    }

    #[test]
    fn parses_note() {
        let note_md = "标题行\n\n正文第一行\n\n正文第三行\n\nid: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\nparent_id: bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\ncreated_time: 2025-01-08T10:02:33.158Z\nupdated_time: 2025-01-08T10:03:18.215Z\nmarkup_language: 1\nis_todo: 0\ntype_: 1";
        let raw = parse_item(note_md).unwrap();
        assert_eq!(raw.item_type(), ItemType::Note);
        let n = to_note(&raw).unwrap();
        assert_eq!(n.title, "标题行");
        // 正文应保留内部空行，且不含标题
        assert_eq!(n.body, "正文第一行\n\n正文第三行");
        assert_eq!(n.parent_id, "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
        assert_eq!(n.created_time, 1736330553158);
        assert_eq!(n.markup_language, MarkupLanguage::Markdown);
    }

    #[test]
    fn parses_note_tag_without_title_or_blank() {
        // note_tag 纯元数据，无标题无空行
        let md = "id: 5828867dfe7f4ce184f3dff4e0e1e756\nnote_id: 8565caad9a4c487996e2e5be171af413\ntag_id: 1434aa3b57b54f839c8a5fc4025cbf10\ntype_: 6";
        let raw = parse_item(md).unwrap();
        assert_eq!(raw.item_type(), ItemType::NoteTag);
        assert!(raw.title.is_none());
        let nt = to_note_tag(&raw).unwrap();
        assert_eq!(nt.note_id, "8565caad9a4c487996e2e5be171af413");
        assert_eq!(nt.tag_id, "1434aa3b57b54f839c8a5fc4025cbf10");
    }

    /// 用真实数据集验证：遍历 JopinData 全部 .md，全部成功解析，类型分布符合预期。
    #[test]
    fn parses_all_real_data() {
        let dir = data_dir();
        if !dir.exists() {
            eprintln!("skipping: test data not found {:?}", dir);
            return;
        }
        let (mut notes, mut folders, mut resources, mut tags, mut note_tags, mut others) =
            (0, 0, 0, 0, 0, 0);
        let mut errors = vec![];
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy();
            // 只认 32hex + .md
            if !(name.len() == 35 && name.ends_with(".md")) {
                continue;
            }
            let content = std::fs::read_to_string(&path).unwrap();
            match parse_item(&content) {
                Ok(raw) => match raw.item_type() {
                    ItemType::Note => {
                        to_note(&raw).unwrap();
                        notes += 1;
                    }
                    ItemType::Folder => {
                        to_folder(&raw).unwrap();
                        folders += 1;
                    }
                    ItemType::Resource => {
                        to_resource(&raw).unwrap();
                        resources += 1;
                    }
                    ItemType::Tag => {
                        to_tag(&raw).unwrap();
                        tags += 1;
                    }
                    ItemType::NoteTag => {
                        to_note_tag(&raw).unwrap();
                        note_tags += 1;
                    }
                    ItemType::Other => others += 1,
                },
                Err(e) => errors.push(format!("{}: {}", name, e)),
            }
        }
        eprintln!(
            "解析结果: 笔记={notes} 笔记本={folders} 资源={resources} 标签={tags} note_tag={note_tags} 其它={others}"
        );
        assert!(errors.is_empty(), "解析失败: {:#?}", errors);
        // 与 grep 统计对齐：343/88/136/3/4，其余(43 revisions)归 Other。
        // 数字跟着 JopinData 的真实内容走——库里增删条目后要照 `grep -l "^type_: N$" *.md | wc -l` 更新。
        assert_eq!(notes, 343);
        assert_eq!(folders, 88);
        assert_eq!(resources, 136);
        assert_eq!(tags, 3);
        assert_eq!(note_tags, 4);
    }
}
