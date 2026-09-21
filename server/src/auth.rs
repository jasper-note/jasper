//! 访问鉴权与授权（access control）。
//!
//! 模型（单一事实来源）：
//! - **未设访问密码** → auth 关闭，一切请求恒为 [`Access::Full`]，行为与无鉴权时完全一致（向后兼容）。
//! - **设了访问密码** → 每次请求落两档 [`Access`]：带有效会话 token 的为 `Full`（可读全部 + 可写），
//!   否则为 `Anonymous`（不可写、不可读机密端点，读的可见范围由 [`Scope`] 决定）。
//!
//! 匿名可见范围 [`Scope`]（[`AuthState::scope`] 求出）：
//! - `passwordless_read` **关** → [`Scope::None`]（什么都看不到，前端显示登录闸门）。
//! - `passwordless_read` **开** → 据笔记本黑白名单 `list_mode` 细化：
//!   `none`=[`Scope::All`]、`whitelist`=[`Scope::Only`]、`blacklist`=[`Scope::Except`]（名单按**子树**展开）。
//!
//! 密码存储：加盐 + 迭代 SHA-256 的哈希（配 config.db，见 [`crate::config::AuthConfig`]），
//! 明文密码永不落库。会话 token：`getrandom` 随机 hex，存**内存** `HashSet`——服务重启即失效、
//! 改密码时全清、登出移除。经 `Authorization: Bearer <token>` 头传递（不用 cookie）。

use crate::config::AuthConfig;
use crate::library::Library;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::RwLock;

/// 密码哈希迭代次数：加盐后反复 SHA-256，拖慢离线暴力破解（登录时算一次，~毫秒级）。
const HASH_ITERS: u32 = 100_000;

/// 一次请求的访问级别。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
	/// 未设密码，或携带有效会话 token：可读全部 + 可写（写仍受全局 `read_only` 约束）。
	Full,
	/// 设了密码但无有效 token：按 `passwordless_read` + 黑白名单决定可见范围，一律不可写。
	Anonymous,
}

/// 匿名请求对笔记本的可见范围。名单已按子树展开成扁平集合。
#[derive(Debug)]
pub enum Scope {
	/// 全部可见（Full，或 passwordless + `list_mode=none`）。
	All,
	/// 什么都看不到（passwordless 关）。
	None,
	/// 仅这些笔记本 id 可见。
	Only(HashSet<String>),
	/// 除这些笔记本 id 外都可见。
	Except(HashSet<String>),
}

impl Scope {
	/// 某笔记本（parent_id）是否对该范围可见。空串=未分类根：whitelist 下不可见、其余按规则。
	pub fn allows_folder(&self, folder_id: &str) -> bool {
		match self {
			Scope::All => true,
			Scope::None => false,
			Scope::Only(set) => set.contains(folder_id),
			Scope::Except(set) => !set.contains(folder_id),
		}
	}
	/// 是否可能有任何可见内容（None 时可短路返回空）。
	pub fn is_none(&self) -> bool {
		matches!(self, Scope::None)
	}
}

/// 运行态鉴权状态：挂在 `AppState`，中间件与读 handler 共用。镜像既有 `read_only: AtomicBool`
/// 的「保存配置时同步运行态」姿势，改设置即时生效无需重启。
pub struct AuthState {
	/// `(salt, hash)` 存在 = 已设密码（受保护）；`None` = 未设密码（auth 关闭，全开放）。
	cred: RwLock<Option<(String, String)>>,
	/// 允许无密码阅读总开关。
	passwordless_read: AtomicBool,
	/// 黑白名单模式：`none` | `whitelist` | `blacklist`。
	list_mode: RwLock<String>,
	/// 名单笔记本 id（未展开子树；`scope()` 现算）。
	folder_list: RwLock<Vec<String>>,
	/// 有效会话 token（内存态）。
	sessions: RwLock<HashSet<String>>,
}

impl AuthState {
	/// 从持久化配置构造（启动时用）。
	pub fn from_config(cfg: &AuthConfig) -> Self {
		let state = AuthState {
			cred: RwLock::new(None),
			passwordless_read: AtomicBool::new(false),
			list_mode: RwLock::new("none".to_string()),
			folder_list: RwLock::new(Vec::new()),
			sessions: RwLock::new(HashSet::new()),
		};
		state.reload(cfg);
		state
	}

	/// 用一份配置覆盖运行态（不动会话——是否清由调用方 [`Self::revoke_all`] 决定）。
	pub fn reload(&self, cfg: &AuthConfig) {
		*self.cred.write().unwrap() = if cfg.password_hash.is_empty() || cfg.password_salt.is_empty() {
			None
		} else {
			Some((cfg.password_salt.clone(), cfg.password_hash.clone()))
		};
		self.passwordless_read.store(cfg.passwordless_read, Ordering::Relaxed);
		*self.list_mode.write().unwrap() = normalize_mode(&cfg.list_mode);
		*self.folder_list.write().unwrap() = cfg.folder_list.clone();
	}

	/// 是否已设密码（受保护）。
	pub fn enabled(&self) -> bool {
		self.cred.read().unwrap().is_some()
	}

	/// 允许无密码阅读总开关。
	pub fn passwordless_read(&self) -> bool {
		self.passwordless_read.load(Ordering::Relaxed)
	}

	/// 校验明文密码（未设密码时恒 false——无需登录）。常量时间比较哈希。
	pub fn verify(&self, password: &str) -> bool {
		let guard = self.cred.read().unwrap();
		match guard.as_ref() {
			Some((salt, hash)) => constant_time_eq(&hash_password(password, salt), hash),
			None => false,
		}
	}

	/// 签发一枚会话 token 并记住它。
	pub fn issue_token(&self) -> String {
		let token = random_hex(32); // 64 hex = 256 bit
		self.sessions.write().unwrap().insert(token.clone());
		token
	}

	/// 校验会话 token 是否有效。
	pub fn valid_token(&self, token: &str) -> bool {
		self.sessions.read().unwrap().contains(token)
	}

	/// 吊销单枚 token（登出）。
	pub fn revoke(&self, token: &str) {
		self.sessions.write().unwrap().remove(token);
	}

	/// 吊销全部会话（改/清密码时用）。
	pub fn revoke_all(&self) {
		self.sessions.write().unwrap().clear();
	}

	/// 据请求携带的 token（可无）定访问级别。auth 关 → 恒 Full。
	pub fn access_for(&self, token: Option<&str>) -> Access {
		if !self.enabled() {
			return Access::Full;
		}
		match token {
			Some(t) if self.valid_token(t) => Access::Full,
			_ => Access::Anonymous,
		}
	}

	/// 据访问级别 + 库结构求匿名可见范围。名单按**子树**展开（复用 `Library::subtree_folder_ids`）。
	pub fn scope(&self, lib: &Library, access: Access) -> Scope {
		if matches!(access, Access::Full) {
			return Scope::All;
		}
		if !self.passwordless_read.load(Ordering::Relaxed) {
			return Scope::None;
		}
		let mode = self.list_mode.read().unwrap().clone();
		if mode == "whitelist" || mode == "blacklist" {
			let list = self.folder_list.read().unwrap();
			let mut set: HashSet<String> = HashSet::new();
			for root in list.iter() {
				set.extend(lib.subtree_folder_ids(root));
			}
			return if mode == "whitelist" { Scope::Only(set) } else { Scope::Except(set) };
		}
		Scope::All // "none" 或未知模式 → 全库可读
	}
}

/// 规整模式取值：只认 whitelist/blacklist，其余归 none。
fn normalize_mode(mode: &str) -> String {
	match mode {
		"whitelist" => "whitelist",
		"blacklist" => "blacklist",
		_ => "none",
	}
	.to_string()
}

/// 加盐 + 迭代 SHA-256：`h0 = sha256(salt || password)`，再迭代 `h = sha256(h)`。
pub fn hash_password(password: &str, salt: &str) -> String {
	use sha2::{Digest, Sha256};
	let mut digest = {
		let mut h = Sha256::new();
		h.update(salt.as_bytes());
		h.update(password.as_bytes());
		h.finalize()
	};
	for _ in 1..HASH_ITERS {
		let mut h = Sha256::new();
		h.update(digest);
		digest = h.finalize();
	}
	hex(&digest)
}

/// 生成一个新盐（32 hex = 128 bit）。
pub fn gen_salt() -> String {
	random_hex(16)
}

/// MCP API key 的固定前缀。让这串东西在配置文件/进程列表/日志里一眼能认出来源，
/// 也方便将来做密钥扫描。
pub const MCP_KEY_PREFIX: &str = "jasper_mcp_";

/// 生成一枚 MCP API key（前缀 + 256 bit 随机 hex）。与会话 token 同强度，
/// 但它是**长效**的：存 config.db，不随服务重启/浏览器登出失效。
pub fn gen_mcp_key() -> String {
	format!("{MCP_KEY_PREFIX}{}", random_hex(32))
}

/// 常数时间比较，避免按字节短路的比较泄露前缀匹配长度。
/// 长度不同直接判否（长度本身不是秘密）。
pub fn secret_eq(a: &str, b: &str) -> bool {
	let (a, b) = (a.as_bytes(), b.as_bytes());
	if a.len() != b.len() {
		return false;
	}
	let mut diff = 0u8;
	for (x, y) in a.iter().zip(b) {
		diff |= x ^ y;
	}
	diff == 0
}

/// n 字节随机数 → 小写 hex。复用 `core::serialize::new_id` 同款 `getrandom` 0.2 自由函数。
fn random_hex(n: usize) -> String {
	let mut bytes = vec![0u8; n];
	getrandom::getrandom(&mut bytes).expect("getrandom 失败");
	hex(&bytes)
}

fn hex(bytes: &[u8]) -> String {
	let mut s = String::with_capacity(bytes.len() * 2);
	for b in bytes {
		s.push_str(&format!("{b:02x}"));
	}
	s
}

/// 常量时间字符串比较（避免按字节短路的时序侧信道）。等长哈希串比较，长度本身非机密。
pub fn constant_time_eq(a: &str, b: &str) -> bool {
	let (a, b) = (a.as_bytes(), b.as_bytes());
	if a.len() != b.len() {
		return false;
	}
	let mut diff = 0u8;
	for (x, y) in a.iter().zip(b.iter()) {
		diff |= x ^ y;
	}
	diff == 0
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::config::AuthConfig;
	use crate::serialize;

	fn cfg(password: Option<&str>, passwordless: bool, mode: &str, list: &[&str]) -> AuthConfig {
		let (salt, hash) = match password {
			Some(p) => {
				let s = gen_salt();
				let h = hash_password(p, &s);
				(s, h)
			}
			None => (String::new(), String::new()),
		};
		AuthConfig {
			password_hash: hash,
			password_salt: salt,
			passwordless_read: passwordless,
			list_mode: mode.to_string(),
			folder_list: list.iter().map(|s| s.to_string()).collect(),
		}
	}

	#[test]
	fn hash_round_trips_and_rejects_wrong() {
		let salt = gen_salt();
		let h = hash_password("open sesame", &salt);
		// 同盐同密码 → 同哈希
		assert_eq!(h, hash_password("open sesame", &salt));
		// 错密码 → 不同哈希
		assert_ne!(h, hash_password("open sesamE", &salt));
		// 换盐 → 不同哈希（防彩虹表）
		assert_ne!(h, hash_password("open sesame", &gen_salt()));
		// 哈希是 64 hex
		assert_eq!(h.len(), 64);
		assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
	}

	#[test]
	fn constant_time_eq_matches_semantics() {
		assert!(constant_time_eq("abc", "abc"));
		assert!(!constant_time_eq("abc", "abd"));
		assert!(!constant_time_eq("abc", "abcd"));
		assert!(constant_time_eq("", ""));
	}

	#[test]
	fn verify_and_sessions() {
		let state = AuthState::from_config(&cfg(Some("s3cret"), false, "none", &[]));
		assert!(state.enabled());
		assert!(state.verify("s3cret"));
		assert!(!state.verify("wrong"));

		// token 唯一、可校验、可吊销
		let t1 = state.issue_token();
		let t2 = state.issue_token();
		assert_ne!(t1, t2);
		assert_eq!(t1.len(), 64);
		assert!(state.valid_token(&t1) && state.valid_token(&t2));
		assert!(!state.valid_token("nope"));
		state.revoke(&t1);
		assert!(!state.valid_token(&t1) && state.valid_token(&t2));
		state.revoke_all();
		assert!(!state.valid_token(&t2));
	}

	#[test]
	fn mcp_key_shape_and_constant_time_compare() {
		let k1 = gen_mcp_key();
		let k2 = gen_mcp_key();
		assert!(k1.starts_with(MCP_KEY_PREFIX));
		assert_eq!(k1.len(), MCP_KEY_PREFIX.len() + 64); // 256 bit hex
		assert_ne!(k1, k2); // 每次都新随机

		assert!(secret_eq(&k1, &k1.clone()));
		assert!(!secret_eq(&k1, &k2));
		assert!(!secret_eq(&k1, "")); // 长度不同
		assert!(!secret_eq(&k1, &k1[..k1.len() - 1])); // 前缀相同但被截短
		assert!(secret_eq("", ""));
	}

	#[test]
	fn access_for_open_vs_protected() {
		// 未设密码 → 恒 Full（即使没 token）
		let open = AuthState::from_config(&cfg(None, false, "none", &[]));
		assert!(!open.enabled());
		assert_eq!(open.access_for(None), Access::Full);

		// 设了密码：无/错 token → Anonymous；有效 token → Full
		let prot = AuthState::from_config(&cfg(Some("pw"), false, "none", &[]));
		assert_eq!(prot.access_for(None), Access::Anonymous);
		assert_eq!(prot.access_for(Some("bad")), Access::Anonymous);
		let t = prot.issue_token();
		assert_eq!(prot.access_for(Some(&t)), Access::Full);
	}

	// 建一个三层库：a(root) > b > c，另有独立 d(root)。用于验证子树可见性。
	fn sample_lib() -> (Library, [String; 4]) {
		let ids: [String; 4] = ["a", "b", "c", "d"].map(|x| x.repeat(32));
		let [a, b, c, d] = ids.clone();
		let contents = vec![
			format!("A\n\nid: {a}\nparent_id: \ntype_: 2"),
			format!("B\n\nid: {b}\nparent_id: {a}\ntype_: 2"),
			format!("C\n\nid: {c}\nparent_id: {b}\ntype_: 2"),
			format!("D\n\nid: {d}\nparent_id: \ntype_: 2"),
			serialize::new_note_md(&"1".repeat(32), &a, "in-a", "x", false, 1),
			serialize::new_note_md(&"2".repeat(32), &c, "in-c", "x", false, 2),
			serialize::new_note_md(&"3".repeat(32), &d, "in-d", "x", false, 3),
		];
		(Library::from_contents(contents).0, ids)
	}

	#[test]
	fn scope_full_and_private() {
		let (lib, _) = sample_lib();
		let prot = AuthState::from_config(&cfg(Some("pw"), false, "none", &[]));
		// Full → 全见
		assert!(matches!(prot.scope(&lib, Access::Full), Scope::All));
		// 匿名 + passwordless 关 → None
		assert!(prot.scope(&lib, Access::Anonymous).is_none());
	}

	#[test]
	fn scope_whitelist_expands_subtree() {
		let (lib, ids) = sample_lib();
		let [a, b, c, d] = ids;
		// 白名单 a → 可见 a,b,c（子树），不含 d
		let st = AuthState::from_config(&cfg(Some("pw"), true, "whitelist", &[&a]));
		let scope = st.scope(&lib, Access::Anonymous);
		assert!(scope.allows_folder(&a));
		assert!(scope.allows_folder(&b));
		assert!(scope.allows_folder(&c));
		assert!(!scope.allows_folder(&d));
		assert!(!scope.allows_folder("")); // 未分类根不在白名单
	}

	#[test]
	fn scope_blacklist_is_complement() {
		let (lib, ids) = sample_lib();
		let [a, b, c, d] = ids;
		// 黑名单 a → 挡掉 a,b,c 子树，其余（d、未分类根）可见
		let st = AuthState::from_config(&cfg(Some("pw"), true, "blacklist", &[&a]));
		let scope = st.scope(&lib, Access::Anonymous);
		assert!(!scope.allows_folder(&a));
		assert!(!scope.allows_folder(&b));
		assert!(!scope.allows_folder(&c));
		assert!(scope.allows_folder(&d));
		assert!(scope.allows_folder("")); // 未分类根不在黑名单 → 可见
	}

	#[test]
	fn scope_passwordless_none_is_all() {
		let (lib, _) = sample_lib();
		let st = AuthState::from_config(&cfg(Some("pw"), true, "none", &[]));
		assert!(matches!(st.scope(&lib, Access::Anonymous), Scope::All));
	}
}
