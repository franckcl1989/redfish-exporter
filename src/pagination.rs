use nv_redfish::Bmc;
use nv_redfish::core::{EntityTypeRef, ODataETag, ODataId};
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;
use url::Url;

/// 单页响应：兼容 `Members@odata.nextLink`（真机）与标准 `@odata.nextLink`。
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

/// 分页上限：防设备返回环状 nextLink 导致死循环。
const MAX_PAGES: usize = 1000;

/// 分页抓取集合，返回全部成员；无 nextLink、遇到重复 URL（设备分页缺陷防循环）或达到上限时停止。
pub async fn fetch_all_pages<B: Bmc>(
    bmc: &Arc<B>,
    url: &ODataId,
) -> Result<Vec<Value>, nv_redfish::Error<B>> {
    let mut out = Vec::new();
    let mut next = url.clone();
    let mut visited = std::collections::HashSet::new();
    visited.insert(next.to_string());
    for _ in 0..MAX_PAGES {
        let page = bmc
            .get::<Page>(&next)
            .await
            .map_err(nv_redfish::Error::Bmc)?;
        out.extend(page.members.iter().cloned());
        let Some(link) = &page.next_link else {
            return Ok(out);
        };
        let Some(resolved) = resolve_next_link(&next, link) else {
            return Ok(out);
        };
        if !visited.insert(resolved.to_string()) {
            tracing::warn!(url = %resolved, "pagination loop detected, stopping");
            return Ok(out);
        }
        next = resolved;
    }
    Ok(out)
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
