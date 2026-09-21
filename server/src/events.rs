//! 变更事件总线：库的每次写入在这里广播，`GET /api/events`（SSE）把它推给前端，
//! 前端按需再拉内容（事件只带 kind/op/id，**不带笔记内容**——保持拉模式的简单性）。
//! 覆盖三类来源：普通 API 写入、插件免确认直写（经 `persist_note_blocking`）、外部 curl 写入。
//! 无订阅者时 send 静默丢弃；接收端落后（lagged）由 SSE 层折算成一条 library reload。

use serde::Serialize;
use tokio::sync::broadcast;

/// 一次库变更。`kind` ∈ note|folder|tag|library；`op` ∈ upsert|delete|reload；
/// `id` 在 kind=library 时为空串（tag 事件的 id 为受影响的笔记 id）。
#[derive(Debug, Clone, Serialize)]
pub struct ChangeEvent {
    pub kind: &'static str,
    pub op: &'static str,
    pub id: String,
}

impl ChangeEvent {
    pub fn reload() -> Self {
        Self { kind: "library", op: "reload", id: String::new() }
    }
}

/// 广播总线（clone 廉价，跨线程安全）。容量之外的旧事件被丢弃——
/// 慢消费者收到 Lagged 后应全量刷新，所以丢事件无害。
#[derive(Clone)]
pub struct EventBus(broadcast::Sender<ChangeEvent>);

impl EventBus {
    pub fn new() -> Self {
        Self(broadcast::channel(256).0)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ChangeEvent> {
        self.0.subscribe()
    }

    fn emit(&self, kind: &'static str, op: &'static str, id: &str) {
        let _ = self.0.send(ChangeEvent { kind, op, id: id.to_string() });
    }

    pub fn note_upserted(&self, id: &str) {
        self.emit("note", "upsert", id);
    }
    pub fn note_deleted(&self, id: &str) {
        self.emit("note", "delete", id);
    }
    pub fn folder_changed(&self, id: &str) {
        self.emit("folder", "upsert", id);
    }
    pub fn folder_deleted(&self, id: &str) {
        self.emit("folder", "delete", id);
    }
    /// 某笔记的标签集变化（打/去标签）。`id` 为受影响的笔记 id，
    /// 前端据此刷新侧栏标签区（含篇数）+ 该笔记打开时的标签行。
    pub fn tags_changed(&self, note_id: &str) {
        self.emit("tag", "upsert", note_id);
    }
    pub fn library_reloaded(&self) {
        let _ = self.0.send(ChangeEvent::reload());
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emits_reach_subscribers_and_no_subscriber_is_fine() {
        let bus = EventBus::new();
        bus.note_upserted("x"); // 无订阅者：不 panic、不报错

        let mut rx = bus.subscribe();
        bus.note_upserted("abc");
        bus.folder_changed("f1");
        bus.note_deleted("abc");
        bus.library_reloaded();

        let ev = rx.try_recv().unwrap();
        assert_eq!((ev.kind, ev.op, ev.id.as_str()), ("note", "upsert", "abc"));
        let ev = rx.try_recv().unwrap();
        assert_eq!((ev.kind, ev.op), ("folder", "upsert"));
        let ev = rx.try_recv().unwrap();
        assert_eq!((ev.kind, ev.op), ("note", "delete"));
        let ev = rx.try_recv().unwrap();
        assert_eq!((ev.kind, ev.op, ev.id.as_str()), ("library", "reload", ""));
    }
}
