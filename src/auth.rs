//! Bearer 认证辅助：常量时间 token 比较与 Authorization 头解析（纯函数，便于单测）。

use axum::http::HeaderMap;

/// 常量时间比较：XOR 折叠遍历到两输入最大长度，长度差异折叠进累加器，
/// 不因长度或内容差异提前退出（防时序侧信道）。空输入相等仅当两者皆空。
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    let mut acc = 0u8;
    for i in 0..a.len().max(b.len()) {
        acc |= a.get(i).copied().unwrap_or(0) ^ b.get(i).copied().unwrap_or(0);
    }
    acc == 0
}

/// 校验请求是否携带合法的 `Authorization: Bearer <expected>`（前缀区分大小写，按 RFC 6750）。
pub fn bearer_authorized(headers: &HeaderMap, expected: &str) -> bool {
    let Some(value) = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    else {
        return false;
    };
    let Some(token) = value.strip_prefix("Bearer ") else {
        return false;
    };
    constant_time_eq(token.as_bytes(), expected.as_bytes())
}
