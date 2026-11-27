//! 网络错误分类：把 reqwest 的错误翻成用户看得懂、且能照着做的一句话。
//!
//! 两件事：
//! 1. 分出「连不上」的原因（被系统拦 / 端口没服务 / 超时 / DNS / TLS），
//!    让用户知道该检查防火墙、DNS 还是代理，而不是只看到一句
//!    「error sending request」。
//! 2. **去掉错误里的 URL**。reqwest 的错误文本带完整请求 URL，而用量端点等
//!    地址是用户自己填的，query 里可能带 token；这些错误会写进活动日志，
//!    不能把凭据带进去。

use reqwest::Error;

/// 连不上的原因。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// 系统拒绝发起连接：多半是防火墙或安全软件拦了本程序（Windows 10013）
    Blocked,
    /// 地址到得了，但端口上没有服务在监听
    Refused,
    /// 超时
    Timeout,
    /// 域名解析失败
    Dns,
    /// HTTPS 握手失败
    Tls,
    /// 其他（原因不明）
    Other,
}

/// 沿 `source()` 链判断连不上的原因。
///
/// 按 `io::ErrorKind` 判断而不是匹配错误文本：系统给的错误文本是本地化的
/// （中文 Windows 上 10013 是「以一种访问权限不允许的方式…」）。DNS 和 TLS
/// 没有对应的 ErrorKind，只能认 hyper / rustls 自己的英文文本。
pub fn classify(error: &Error) -> Kind {
    if error.is_timeout() {
        return Kind::Timeout;
    }
    classify_chain(error)
}

/// 与 [`classify`] 同一套判定，但接受任意错误（便于单测构造错误链）。
pub fn classify_chain(error: &(dyn std::error::Error + 'static)) -> Kind {
    let (mut dns, mut blocked, mut refused, mut timeout, mut tls) =
        (false, false, false, false, false);
    let mut current = Some(error);
    while let Some(cause) = current {
        if let Some(io) = cause.downcast_ref::<std::io::Error>() {
            match io.kind() {
                std::io::ErrorKind::PermissionDenied => blocked = true,
                std::io::ErrorKind::ConnectionRefused => refused = true,
                std::io::ErrorKind::TimedOut => timeout = true,
                _ => {}
            }
        }
        let text = cause.to_string().to_ascii_lowercase();
        if text.starts_with("dns error") || text.contains("failed to lookup address") {
            dns = true;
        }
        if text.contains("certificate") || text.contains("tls") || text.contains("handshake") {
            tls = true;
        }
        current = cause.source();
    }
    if dns {
        Kind::Dns
    } else if blocked {
        Kind::Blocked
    } else if refused {
        Kind::Refused
    } else if timeout {
        Kind::Timeout
    } else if tls {
        Kind::Tls
    } else {
        Kind::Other
    }
}

/// 一句能照着做的提示。
pub fn hint(kind: Kind) -> &'static str {
    match kind {
        Kind::Blocked => "连接被系统拒绝：可能被防火墙或安全软件拦截，请允许本程序联网后重试",
        Kind::Refused => "地址能解析，但该端口上没有服务在监听",
        Kind::Timeout => "连接超时：对方长时间没有响应（可能被网络或代理拖住）",
        Kind::Dns => "域名解析失败：检查网络连接与 DNS / 代理设置",
        Kind::Tls => "HTTPS 握手失败：证书或代理配置可能有问题",
        Kind::Other => "请求失败",
    }
}

/// 组装提示语：原文只作补充，且已由调用方去掉 URL。
pub fn message(kind: Kind, detail: &str) -> String {
    let hint = hint(kind);
    if detail.trim().is_empty() {
        hint.to_string()
    } else {
        format!("{hint}（{detail}）")
    }
}

/// 网络错误的完整描述：分类 + 原文（**不含 URL**）。
///
/// 按值接收：`without_url()` 会消耗掉错误本身（reqwest 的错误里存着请求 URL）。
pub fn describe(error: Error) -> String {
    let kind = classify(&error);
    message(kind, &error.without_url().to_string())
}

#[cfg(test)]
mod tests {
    // 不用 super::*：它会把 reqwest::Error 带进来，和 std::error::Error 撞名
    use super::{classify_chain, describe, message, Kind};
    use std::error::Error as StdError;
    use std::io::ErrorKind;
    use std::time::Duration;

    /// 外层文本可指定的错误，模拟 hyper_util 的 ConnectError（「tcp connect error」）
    #[derive(Debug)]
    struct Labeled(&'static str, Option<std::io::Error>);

    impl std::fmt::Display for Labeled {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(self.0)
        }
    }

    impl StdError for Labeled {
        fn source(&self) -> Option<&(dyn StdError + 'static)> {
            self.1.as_ref().map(|e| e as &(dyn StdError + 'static))
        }
    }

    fn io(kind: ErrorKind, text: &str) -> Option<std::io::Error> {
        Some(std::io::Error::new(kind, text.to_string()))
    }

    /// 按 io::ErrorKind 判断，不受本地化错误文本影响
    #[test]
    fn classify_reads_io_kind_not_localized_text() {
        let cases = [
            (
                Labeled(
                    "tcp connect error",
                    io(
                        ErrorKind::PermissionDenied,
                        "以一种访问权限不允许的方式做了一个访问套接字的尝试。 (os error 10013)",
                    ),
                ),
                Kind::Blocked,
            ),
            (
                Labeled("tcp connect error", io(ErrorKind::ConnectionRefused, "x")),
                Kind::Refused,
            ),
            (
                Labeled("tcp connect error", io(ErrorKind::TimedOut, "x")),
                Kind::Timeout,
            ),
            (Labeled("dns error", io(ErrorKind::Other, "x")), Kind::Dns),
            (
                Labeled("invalid peer certificate: UnknownIssuer", None),
                Kind::Tls,
            ),
            (
                Labeled("connection closed before message completed", None),
                Kind::Other,
            ),
        ];
        for (err, want) in cases {
            assert_eq!(classify_chain(&err), want, "{err}");
        }
    }

    #[test]
    fn message_hints_per_kind() {
        assert!(message(Kind::Blocked, "").contains("防火墙"));
        assert!(message(Kind::Refused, "").contains("没有服务"));
        assert!(message(Kind::Timeout, "").contains("超时"));
        assert!(message(Kind::Dns, "").contains("DNS"));
        assert!(message(Kind::Tls, "").contains("握手"));
        assert_eq!(message(Kind::Other, ""), "请求失败");
        // 原文作补充
        assert!(message(Kind::Other, "boom").contains("boom"));
    }

    /// 提示语不泄露 URL：用量端点等是用户填的地址，query 里可能带 token，
    /// 而这些错误会写进活动日志。用本地回环地址构造失败，不产生外部流量。
    #[test]
    fn describe_strips_url() {
        let err = tauri::async_runtime::block_on(async {
            reqwest::Client::new()
                .get("http://127.0.0.1:9/usage?token=secret-token-value")
                .timeout(Duration::from_secs(3))
                .send()
                .await
                .expect_err("回环端口上不会有服务在监听")
        });
        let text = describe(err);
        assert!(!text.contains("secret-token-value"), "{text}");
        assert!(!text.contains("127.0.0.1"), "{text}");
    }
}
