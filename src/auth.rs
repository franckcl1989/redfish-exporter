//! Bearer 认证辅助：常量时间 token 比较与 Authorization 头解析（纯函数，便于单测）。

use axum::http::HeaderMap;

/// 常量时间比较：XOR 折叠遍历到两输入最大长度，长度差异折叠进累加器，
/// 不因长度或内容差异提前退出（防时序侧信道）。空输入相等仅当两者皆空。
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    let mut acc = a.len() ^ b.len();
    for i in 0..a.len().max(b.len()) {
        acc |= usize::from(a.get(i).copied().unwrap_or(0) ^ b.get(i).copied().unwrap_or(0));
    }
    acc == 0
}

/// 校验请求是否携带合法的 `Authorization: Bearer <expected>`。
/// HTTP 认证 scheme 不区分大小写；token 本身按字节精确比较。
pub fn bearer_authorized(headers: &HeaderMap, expected: &str) -> bool {
    let Some(value) = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    else {
        return false;
    };
    let Some((scheme, token)) = value.split_once(' ') else {
        return false;
    };
    if !scheme.eq_ignore_ascii_case("Bearer") {
        return false;
    }
    let token = token.trim_start_matches(' ');
    constant_time_eq(token.as_bytes(), expected.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::constant_time_eq;

    #[test]
    fn constant_time_comparison_includes_length() {
        assert!(constant_time_eq(b"same", b"same"));
        assert!(!constant_time_eq(b"same", b"same\0"));
        assert!(!constant_time_eq(b"same", b"different"));
    }
}
