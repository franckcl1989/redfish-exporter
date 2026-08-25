use nv_redfish::Bmc;
use nv_redfish::core::{EntityTypeRef, ODataETag, ODataId};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use url::Url;

/// 单页响应：兼容 `Members@odata.nextLink`（真机）与标准 `@odata.nextLink`。
#[derive(Serialize)]
struct Page {
    id: ODataId,
    members: Vec<Value>,
    next_link: Option<String>,
}

impl<'de> Deserialize<'de> for Page {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        let id = value
            .get("@odata.id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
            .into();
        let members = value
            .get("Members")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let next_link = value
            .get("Members@odata.nextLink")
            .or_else(|| value.get("@odata.nextLink"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        Ok(Self {
            id,
            members,
            next_link,
        })
    }
}

impl EntityTypeRef for Page {
    fn odata_id(&self) -> &ODataId {
        &self.id
    }

    fn etag(&self) -> Option<&ODataETag> {
        None
    }
}

/// 分页页数上限：防设备返回环状 nextLink（visited 去重为主防线，此为第二防线）。
pub const MAX_PAGES: usize = 1000;
/// 单页响应体上限（64MiB）：防异常/恶意 BMC 超大响应耗尽内存。
/// 说明：nv-redfish 类型化 fetch 不提供流式读取（Bmc trait 无 raw get），
/// 上限以反序列化后序列化字节数近似校验（与真实响应同量级），超限即报错丢弃，
/// 内存峰值受限于单页；流式上限记入 backlog（security.md 如实文档化）。
pub const MAX_PAGE_BYTES: usize = 64 * 1024 * 1024;
/// 累计成员上限：防无 nextLink 变化的设备无限积累（真机最大 2064 条，留 100 倍余量）。
pub const MAX_TOTAL_MEMBERS: usize = 200_000;

/// 分页失败：BMC 请求错误 / 单页超限 / 页数超限 / 成员累计超限。
/// 说明：`Bmc` 变体不能使用 `#[from]`——`From<B::Error>` 与核心库
/// `impl<T> From<T> for T` 存在一致性冲突（关联类型对 coherence 不透明），
/// 故在调用处显式 `map_err(PaginationError::Bmc)` 转换。
#[derive(thiserror::Error)]
pub enum PaginationError<B: Bmc> {
    #[error("bmc request failed: {0}")]
    Bmc(B::Error),
    #[error("pagination page exceeds size limit")]
    PageTooLarge,
    #[error("pagination exceeded {0} pages")]
    TooManyPages(usize),
    #[error("pagination exceeded {0} total members")]
    TooManyMembers(usize),
}

// 手写 Debug：derive 会对泛型参数 `B` 加 `B: Debug` 约束（字段中提及即约束），
// 而 nv-redfish-bmc-mock 的 Bmc 未实现 Debug，导致既有测试 `.unwrap()` 无法编译。
impl<B: Bmc> std::fmt::Debug for PaginationError<B> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Bmc(e) => f.debug_tuple("Bmc").field(e).finish(),
            Self::PageTooLarge => f.write_str("PageTooLarge"),
            Self::TooManyPages(n) => f.debug_tuple("TooManyPages").field(n).finish(),
            Self::TooManyMembers(n) => f.debug_tuple("TooManyMembers").field(n).finish(),
        }
    }
}

pub async fn fetch_all_pages<B: Bmc>(
    bmc: &Arc<B>,
    url: &ODataId,
) -> Result<Vec<Value>, PaginationError<B>> {
    fetch_all_pages_with_limits(bmc, url, MAX_PAGES, MAX_PAGE_BYTES, MAX_TOTAL_MEMBERS).await
}

/// 带显式上限的分页抓取（测试接缝）：上限语义同 fetch_all_pages。
pub async fn fetch_all_pages_with_limits<B: Bmc>(
    bmc: &Arc<B>,
    url: &ODataId,
    max_pages: usize,
    max_page_bytes: usize,
    max_total_members: usize,
) -> Result<Vec<Value>, PaginationError<B>> {
    let mut out = Vec::new();
    let mut next = url.clone();
    let mut visited = std::collections::HashSet::new();
    visited.insert(next.to_string());
    for _ in 0..max_pages {
        let page = bmc.get::<Page>(&next).await.map_err(PaginationError::Bmc)?;
        let size = serde_json::to_vec(&*page)
            .map(|v| v.len())
            .unwrap_or(usize::MAX);
        if size > max_page_bytes {
            tracing::warn!(url = %next, size, "pagination page exceeds size limit");
            return Err(PaginationError::PageTooLarge);
        }
        out.extend(page.members.iter().cloned());
        if out.len() > max_total_members {
            tracing::warn!(
                url = %next,
                members = out.len(),
                "pagination member accumulation limit reached"
            );
            return Err(PaginationError::TooManyMembers(max_total_members));
        }
        let Some(link) = &page.next_link else {
            return Ok(out);
        };
        let Some(resolved) = resolve_next_link(&next, link) else {
            tracing::warn!(url = %next, link, "pagination nextLink resolve failed, stopping");
            return Ok(out);
        };
        if !visited.insert(resolved.to_string()) {
            tracing::warn!(url = %resolved, "pagination loop detected, stopping");
            return Ok(out);
        }
        next = resolved;
    }
    tracing::warn!(url = %next, "pagination page limit reached");
    Err(PaginationError::TooManyPages(max_pages))
}

/// 解析 nextLink：以 `/` 开头的相对路径直接采用；绝对 http(s) 仅放行同源。
fn resolve_next_link(current: &ODataId, link: &str) -> Option<ODataId> {
    if link.starts_with('/') {
        return Some(link.to_string().into());
    }
    let url = Url::parse(link).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let cur = Url::parse(&current.to_string()).ok()?;
    if (cur.scheme(), cur.host_str(), cur.port()) != (url.scheme(), url.host_str(), url.port()) {
        return None;
    }
    Some(link.to_string().into())
}
