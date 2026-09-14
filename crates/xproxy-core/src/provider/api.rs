//! Gọi API nhà cung cấp: một lần GET URL → một dòng proxy → gán vào cổng.

use std::time::Duration;

use reqwest::Url;

use crate::error::{Error, Result};
use crate::relay::Upstream;

use super::{LineFormat, Provider};

fn client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        // Mỗi lần lấy IP là một kết nối mới. Tái dùng socket cũ với vài nhà cung cấp
        // sẽ trả về đúng IP của lần trước → người dùng tưởng "bấm Đổi mà IP không đổi".
        .pool_max_idle_per_host(0)
        .build()?)
}

/// Những thứ app ghi đè lên URL người dùng dán.
#[derive(Debug, Clone, Default)]
pub struct UrlOverrides {
    pub country: String,
    pub state: String,
    pub city: String,
    /// Giá trị cho tham số `port` của nhà cung cấp (nếu URL có tham số đó).
    pub provider_port: Option<u16>,
}

/// Dựng URL thật từ URL người dùng dán.
///
/// Quy tắc cố ý giữ đơn giản và không phá URL gốc:
/// - `num` luôn bị ép về `1` — một lần Gán = một IP, khỏi tiêu nhầm hạn mức.
/// - `country` / `state` / `city`: chỉ ghi đè khi người dùng có chọn ở UI. Bỏ trống
///   thì giữ nguyên những gì URL đã có.
/// - `port`: chỉ đổi khi URL vốn đã có tham số đó VÀ nhà cung cấp bật `vary_port`.
///   Không tự ý thêm tham số lạ vào URL của hãng.
pub fn build_url(template: &str, o: &UrlOverrides, vary_port: bool) -> Result<Url> {
    let t = template.trim();
    if t.is_empty() {
        return Err(Error::Api("chưa dán URL lấy IP cho nhà cung cấp này".into()));
    }
    let url = Url::parse(t).map_err(|e| Error::Api(format!("URL không hợp lệ: {e}")))?;

    let pairs: Vec<(String, String)> = url
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    let has = |name: &str| pairs.iter().any(|(k, _)| k == name);

    let mut out = url.clone();
    {
        let mut q = out.query_pairs_mut();
        q.clear();
        let mut wrote_num = false;

        for (k, v) in &pairs {
            let replacement = match k.as_str() {
                "num" => {
                    wrote_num = true;
                    Some("1".to_string())
                }
                "country" if !o.country.is_empty() => Some(o.country.clone()),
                "state" if !o.state.is_empty() => Some(o.state.clone()),
                "city" if !o.city.is_empty() => Some(o.city.clone()),
                "port" => match (vary_port, o.provider_port) {
                    (true, Some(p)) => Some(p.to_string()),
                    _ => None,
                },
                _ => None,
            };
            q.append_pair(k, replacement.as_deref().unwrap_or(v));
        }

        // Thiếu `num` thì thêm — đây là tham số duy nhất đáng tự thêm, vì nó chặn
        // việc một lần bấm Gán vô tình tiêu nhiều IP.
        if !wrote_num {
            q.append_pair("num", "1");
        }
        // Có chọn vùng mà URL chưa có tham số tương ứng thì thêm vào.
        if !o.country.is_empty() && !has("country") {
            q.append_pair("country", &o.country);
        }
        if !o.state.is_empty() && !has("state") {
            q.append_pair("state", &o.state);
        }
        if !o.city.is_empty() && !has("city") {
            q.append_pair("city", &o.city);
        }
    }
    Ok(out)
}

/// Parse một dòng proxy. Không khớp dạng ưu tiên thì tự thử dạng còn lại.
pub fn parse_line(line: &str, prefer: LineFormat) -> Option<Upstream> {
    let t = line.trim();
    if t.is_empty() {
        return None;
    }

    let cred_at_host = || -> Option<Upstream> {
        let (cred, hostport) = t.split_once('@')?;
        let (user, pass) = cred.split_once(':')?;
        let (host, port) = hostport.rsplit_once(':')?;
        if !host.contains('.') && !host.contains(':') {
            return None;
        }
        Some(Upstream::new(host, port.trim().parse().ok()?).with_auth(user, pass))
    };

    let host_first = || -> Option<Upstream> {
        // splitn(4) nên mật khẩu chứa dấu ':' vẫn giữ nguyên.
        let parts: Vec<&str> = t.splitn(4, ':').collect();
        if parts.len() != 4 || !parts[0].contains('.') {
            return None;
        }
        Some(Upstream::new(parts[0], parts[1].trim().parse().ok()?).with_auth(parts[2], parts[3]))
    };

    match prefer {
        LineFormat::CredAtHost => cred_at_host().or_else(host_first),
        LineFormat::HostFirst => host_first().or_else(cred_at_host),
    }
}

pub fn parse_upstreams(text: &str, prefer: LineFormat) -> Vec<Upstream> {
    text.lines().filter_map(|l| parse_line(l, prefer)).collect()
}

fn preview(text: &str) -> String {
    let t = text.trim();
    if t.chars().count() > 200 {
        format!("{}…", t.chars().take(200).collect::<String>())
    } else {
        t.to_string()
    }
}

pub async fn fetch_upstreams(p: &Provider, o: &UrlOverrides) -> Result<Vec<Upstream>> {
    let url = build_url(&p.url, o, p.vary_port)?;
    let resp = client()?.get(url).send().await?;
    let status = resp.status().as_u16();
    let text = resp.text().await?;

    if text.trim().is_empty() {
        return Err(Error::Api(format!("{} trả về rỗng [HTTP {status}]", p.name)));
    }
    let ups = parse_upstreams(&text, p.line_format);
    if ups.is_empty() {
        return Err(Error::Api(format!(
            "{} trả về [HTTP {status}]: {}",
            p.name,
            preview(&text)
        )));
    }
    Ok(ups
        .into_iter()
        .map(|mut u| {
            u.scheme = p.upstream_scheme;
            u
        })
        .collect())
}

/// Lấy đúng một IP — cái app dùng khi người dùng bấm Gán / Đổi.
pub async fn fetch_one(p: &Provider, o: &UrlOverrides) -> Result<Upstream> {
    fetch_upstreams(p, o)
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| Error::Api("không có IP nào trả về".into()))
}

/// GET thô, cho nút "Test" — trả cả URL đã gọi lẫn phản hồi nguyên văn.
pub async fn raw_get(url: &str) -> (String, String) {
    let c = match client() {
        Ok(c) => c,
        Err(e) => return (url.to_string(), format!("lỗi tạo client: {e}")),
    };
    match c.get(url).send().await {
        Ok(resp) => {
            let code = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            (
                format!("{url}   [HTTP {code}]"),
                if body.trim().is_empty() { "(rỗng)".into() } else { body },
            )
        }
        Err(e) => (url.to_string(), format!("lỗi mạng: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLI: &str = "https://webipapi.cliproxy.com/api/getIpInfo?key=kd7c2apyfaxslfb61syy&port=443&num=1&country=US&state=&type=2";

    #[test]
    fn parse_dung_dong_that_cua_cliproxy() {
        let u = parse_line(
            "64.205.177.44:443:6ab9171c2721211f:kd7c2apyfaxslfb61syy",
            LineFormat::HostFirst,
        )
        .unwrap();
        assert_eq!(u.host, "64.205.177.44");
        assert_eq!(u.port, 443);
        assert_eq!(u.user, "6ab9171c2721211f");
        assert_eq!(u.pass, "kd7c2apyfaxslfb61syy");
    }

    #[test]
    fn parse_dang_cred_at_host() {
        let u = parse_line("user:pass@gate.example.com:7000", LineFormat::CredAtHost).unwrap();
        assert_eq!(u.host, "gate.example.com");
        assert_eq!(u.port, 7000);
    }

    #[test]
    fn mat_khau_co_dau_hai_cham_van_nguyen_ven() {
        let u = parse_line("1.2.3.4:1080:bob:a:b:c", LineFormat::HostFirst).unwrap();
        assert_eq!(u.pass, "a:b:c");
    }

    #[test]
    fn dan_nham_dinh_dang_van_nhan_ra() {
        let u = parse_line("u:p@1.2.3.4:1080", LineFormat::HostFirst).unwrap();
        assert_eq!(u.host, "1.2.3.4");
        assert_eq!(u.user, "u");
    }

    #[test]
    fn bo_qua_dong_rac_trong_phan_hoi() {
        let text = "\n64.205.177.44:443:u:p\nSorry, no proxy available\n1.1.1.1:80:a:b\n";
        assert_eq!(parse_upstreams(text, LineFormat::HostFirst).len(), 2);
    }

    /// URL dán vào phải giữ nguyên key và các tham số lạ của hãng.
    #[test]
    fn giu_nguyen_url_goc() {
        let u = build_url(CLI, &UrlOverrides::default(), true).unwrap();
        let s = u.as_str();
        assert!(s.contains("key=kd7c2apyfaxslfb61syy"));
        assert!(s.contains("type=2"));
        assert!(s.starts_with("https://webipapi.cliproxy.com/api/getIpInfo?"));
    }

    /// `num` luôn bị ép về 1 để một lần bấm Gán không tiêu nhiều IP.
    #[test]
    fn luon_ep_num_bang_1() {
        let url = CLI.replace("num=1", "num=25");
        let u = build_url(&url, &UrlOverrides::default(), true).unwrap();
        assert!(u.as_str().contains("num=1"));
        assert!(!u.as_str().contains("num=25"));
    }

    /// Chọn vùng ở UI thì ghi đè; bỏ trống thì giữ nguyên URL.
    #[test]
    fn ghi_de_vung_khi_co_chon() {
        let o = UrlOverrides { country: "GB".into(), state: "Texas".into(), ..Default::default() };
        let u = build_url(CLI, &o, true).unwrap();
        assert!(u.as_str().contains("country=GB"));
        assert!(u.as_str().contains("state=Texas"));

        let u = build_url(CLI, &UrlOverrides::default(), true).unwrap();
        assert!(u.as_str().contains("country=US"), "bỏ trống thì giữ nguyên URL");
    }

    #[test]
    fn doi_tham_so_port_theo_cong() {
        let o = UrlOverrides { provider_port: Some(451), ..Default::default() };
        assert!(build_url(CLI, &o, true).unwrap().as_str().contains("port=451"));
        assert!(build_url(CLI, &o, false).unwrap().as_str().contains("port=443"));
    }

    /// URL không có tham số `port` thì đừng tự thêm vào — dễ làm hỏng API của hãng khác.
    #[test]
    fn khong_tu_them_port_la() {
        let o = UrlOverrides { provider_port: Some(451), ..Default::default() };
        let u = build_url("https://x.example/get?key=abc", &o, true).unwrap();
        assert!(!u.as_str().contains("port="));
    }

    #[test]
    fn url_rong_bao_loi_ro_rang() {
        let e = build_url("  ", &UrlOverrides::default(), true).unwrap_err();
        assert!(e.to_string().contains("chưa dán URL"));
    }
}
