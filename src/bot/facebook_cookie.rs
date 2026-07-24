use std::collections::BTreeMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use reqwest::header::{COOKIE, HeaderMap, LOCATION, REFERER, SET_COOKIE};
use reqwest::{Client, Proxy, Response, StatusCode, redirect::Policy};
use url::Url;

const FACEBOOK_HOME_URL: &str = "https://m.facebook.com/";
const FACEBOOK_LOGIN_URL: &str = "https://m.facebook.com/login.php";
const FACEBOOK_USER_AGENT: &str = "Mozilla/5.0 (Linux; Android 13; Pixel 7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0 Mobile Safari/537.36";
const MAX_REDIRECTS: usize = 8;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FacebookCookieInput {
    pub uid: Option<String>,
    pub password: Option<String>,
    pub two_fa: Option<String>,
    pub cookie: Option<String>,
}

impl FacebookCookieInput {
    pub fn parse(raw: &str) -> Result<Self> {
        let raw = raw.trim();
        if raw.is_empty() {
            bail!("Vui lòng gửi UID|PASS|2FA|COOKIE, UID|PASS|2FA hoặc COOKIE.");
        }

        let parts = raw.splitn(4, '|').map(str::trim).collect::<Vec<_>>();
        let looks_like_credentials = parts.len() >= 3
            && !parts[0].is_empty()
            && !parts[1].is_empty()
            && !parts[0].contains('=');

        if looks_like_credentials {
            let cookie = parts
                .get(3)
                .copied()
                .filter(|value| !value.is_empty())
                .map(str::to_string);
            if let Some(cookie) = cookie.as_deref()
                && !looks_like_facebook_cookie(cookie)
            {
                bail!("Cookie Facebook cần có c_user hoặc xs.");
            }

            return Ok(Self {
                uid: Some(parts[0].to_string()),
                password: Some(parts[1].to_string()),
                two_fa: parts
                    .get(2)
                    .copied()
                    .filter(|value| !value.is_empty())
                    .map(str::to_string),
                cookie,
            });
        }

        if looks_like_facebook_cookie(raw) {
            return Ok(Self {
                uid: None,
                password: None,
                two_fa: None,
                cookie: Some(raw.to_string()),
            });
        }

        bail!("Sai định dạng. Hãy gửi UID|PASS|2FA|COOKIE, UID|PASS|2FA hoặc COOKIE.")
    }

    fn credentials(&self) -> Option<(&str, &str)> {
        Some((self.uid.as_deref()?, self.password.as_deref()?))
    }
}

#[derive(Debug)]
struct Page {
    url: Url,
    status: StatusCode,
    location: Option<String>,
    body: String,
}

#[derive(Debug)]
struct HtmlForm {
    action: String,
    fields: Vec<(String, String)>,
    html: String,
}

pub async fn get_live_cookie(
    input: &FacebookCookieInput,
    proxy_url: Option<&str>,
) -> Result<String> {
    let mut builder = Client::builder()
        .redirect(Policy::none())
        .timeout(Duration::from_secs(35))
        .user_agent(FACEBOOK_USER_AGENT);
    if let Some(proxy_url) = proxy_url.map(str::trim).filter(|value| !value.is_empty()) {
        builder = builder.proxy(
            Proxy::all(proxy_url).context("Proxy lấy cookie Facebook không hợp lệ")?,
        );
    }
    let client = builder.build()?;

    let mut cookies = input
        .cookie
        .as_deref()
        .map(parse_cookie_header)
        .unwrap_or_default();
    if input.cookie.is_some() {
        if facebook_session_is_live(&client, &mut cookies).await? {
            return Ok(cookie_header(&cookies));
        }
        if input.credentials().is_none() {
            bail!("Cookie đã hết hạn hoặc tài khoản đang checkpoint. Hãy gửi thêm UID|PASS|2FA.");
        }
    }

    let Some((uid, password)) = input.credentials() else {
        bail!("Không tìm thấy UID và mật khẩu để đăng nhập lại.");
    };

    for session_cookie in ["c_user", "xs", "presence"] {
        cookies.remove(session_cookie);
    }
    login_with_credentials(
        &client,
        &mut cookies,
        uid,
        password,
        input.two_fa.as_deref(),
    )
    .await?;

    if !has_login_cookies(&cookies) {
        bail!("Facebook không trả về cookie đăng nhập hợp lệ.");
    }
    Ok(cookie_header(&cookies))
}

async fn facebook_session_is_live(
    client: &Client,
    cookies: &mut BTreeMap<String, String>,
) -> Result<bool> {
    if !has_login_cookies(cookies) {
        return Ok(false);
    }
    let page = follow_get_redirects(
        client,
        get_page(client, FACEBOOK_HOME_URL, cookies, None).await?,
        cookies,
    )
    .await?;
    Ok(page_is_logged_in(&page, cookies))
}

async fn login_with_credentials(
    client: &Client,
    cookies: &mut BTreeMap<String, String>,
    uid: &str,
    password: &str,
    two_fa: Option<&str>,
) -> Result<()> {
    let login_page = follow_get_redirects(
        client,
        get_page(client, FACEBOOK_LOGIN_URL, cookies, None).await?,
        cookies,
    )
    .await?;
    let login_form = find_form(&login_page.body, "email")
        .or_else(|| find_form(&login_page.body, "pass"))
        .ok_or_else(|| anyhow!("Không tìm thấy biểu mẫu đăng nhập Facebook."))?;
    let login_url = resolve_facebook_url(&login_page.url, &login_form.action)?;
    let mut fields = login_form.fields;
    upsert_field(&mut fields, "email", uid);
    upsert_field(&mut fields, "pass", password);
    upsert_field(&mut fields, "login", "Log In");

    let mut page = follow_get_redirects(
        client,
        post_form(
            client,
            login_url.as_str(),
            cookies,
            Some(login_page.url.as_str()),
            &fields,
        )
        .await?,
        cookies,
    )
    .await?;

    for _ in 0..5 {
        if page_is_logged_in(&page, cookies) {
            return Ok(());
        }
        if page_requires_approval_code(&page) {
            let two_fa = two_fa.ok_or_else(|| {
                anyhow!("Tài khoản yêu cầu 2FA. Hãy gửi UID|PASS|2FA để bot đăng nhập.")
            })?;
            let code = current_two_factor_code(two_fa)?;
            let form = find_form(&page.body, "approvals_code")
                .ok_or_else(|| anyhow!("Không tìm thấy biểu mẫu nhập mã 2FA Facebook."))?;
            let action = resolve_facebook_url(&page.url, &form.action)?;
            let mut fields = form.fields;
            upsert_field(&mut fields, "approvals_code", &code);
            upsert_field(&mut fields, "submit[Submit Code]", "Submit Code");
            page = follow_get_redirects(
                client,
                post_form(
                    client,
                    action.as_str(),
                    cookies,
                    Some(page.url.as_str()),
                    &fields,
                )
                .await?,
                cookies,
            )
            .await?;
            continue;
        }

        if let Some(form) = find_form(&page.body, "name_action_selected") {
            let action = resolve_facebook_url(&page.url, &form.action)?;
            let mut fields = form.fields;
            let action_value = preferred_action_value(&form.html)
                .unwrap_or_else(|| "save_device".to_string());
            upsert_field(&mut fields, "name_action_selected", &action_value);
            upsert_field(&mut fields, "submit[Continue]", "Continue");
            page = follow_get_redirects(
                client,
                post_form(
                    client,
                    action.as_str(),
                    cookies,
                    Some(page.url.as_str()),
                    &fields,
                )
                .await?,
                cookies,
            )
            .await?;
            continue;
        }

        if let Some(form) = find_form(&page.body, "submit[Continue]") {
            let action = resolve_facebook_url(&page.url, &form.action)?;
            let mut fields = form.fields;
            upsert_field(&mut fields, "submit[Continue]", "Continue");
            page = follow_get_redirects(
                client,
                post_form(
                    client,
                    action.as_str(),
                    cookies,
                    Some(page.url.as_str()),
                    &fields,
                )
                .await?,
                cookies,
            )
            .await?;
            continue;
        }
        break;
    }

    if page_is_logged_in(&page, cookies) {
        return Ok(());
    }
    let lower = page.body.to_ascii_lowercase();
    if lower.contains("checkpoint") || page.url.as_str().contains("checkpoint") {
        bail!("Tài khoản đang checkpoint hoặc cần xác minh thêm trên Facebook.");
    }
    if lower.contains("incorrect password")
        || lower.contains("mật khẩu bạn đã nhập không chính xác")
        || lower.contains("the password you entered is incorrect")
    {
        bail!("UID hoặc mật khẩu Facebook không chính xác.");
    }
    bail!("Đăng nhập Facebook thất bại. Hãy kiểm tra UID, mật khẩu và 2FA rồi thử lại.")
}

async fn get_page(
    client: &Client,
    url: &str,
    cookies: &mut BTreeMap<String, String>,
    referer: Option<&str>,
) -> Result<Page> {
    ensure_facebook_url(url)?;
    let mut request = client.get(url);
    if !cookies.is_empty() {
        request = request.header(COOKIE, cookie_header(cookies));
    }
    if let Some(referer) = referer {
        request = request.header(REFERER, referer);
    }
    response_page(request.send().await?, cookies).await
}

async fn post_form(
    client: &Client,
    url: &str,
    cookies: &mut BTreeMap<String, String>,
    referer: Option<&str>,
    fields: &[(String, String)],
) -> Result<Page> {
    ensure_facebook_url(url)?;
    let mut request = client.post(url).form(fields);
    if !cookies.is_empty() {
        request = request.header(COOKIE, cookie_header(cookies));
    }
    if let Some(referer) = referer {
        request = request.header(REFERER, referer);
    }
    response_page(request.send().await?, cookies).await
}

async fn response_page(
    response: Response,
    cookies: &mut BTreeMap<String, String>,
) -> Result<Page> {
    merge_set_cookies(cookies, response.headers());
    let url = response.url().clone();
    ensure_facebook_url(url.as_str())?;
    let status = response.status();
    let location = response
        .headers()
        .get(LOCATION)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let body = response.text().await.unwrap_or_default();
    Ok(Page {
        url,
        status,
        location,
        body,
    })
}

async fn follow_get_redirects(
    client: &Client,
    mut page: Page,
    cookies: &mut BTreeMap<String, String>,
) -> Result<Page> {
    for _ in 0..MAX_REDIRECTS {
        if !page.status.is_redirection() {
            return Ok(page);
        }
        let location = page
            .location
            .as_deref()
            .ok_or_else(|| anyhow!("Facebook chuyển hướng nhưng không trả URL."))?;
        let next_url = resolve_facebook_url(&page.url, location)?;
        page = get_page(client, next_url.as_str(), cookies, Some(page.url.as_str())).await?;
    }
    bail!("Facebook chuyển hướng quá nhiều lần.")
}

fn page_is_logged_in(page: &Page, cookies: &BTreeMap<String, String>) -> bool {
    if !page.status.is_success() || !has_login_cookies(cookies) {
        return false;
    }
    let url = page.url.as_str().to_ascii_lowercase();
    let body = page.body.to_ascii_lowercase();
    !url.contains("/login")
        && !url.contains("checkpoint")
        && !body.contains("name=\"approvals_code\"")
        && !body.contains("name='approvals_code'")
        && !body.contains("name=\"email\"")
}

fn page_requires_approval_code(page: &Page) -> bool {
    let body = page.body.to_ascii_lowercase();
    body.contains("approvals_code") || body.contains("login approval")
}

fn find_form(html: &str, required_field: &str) -> Option<HtmlForm> {
    let lower = html.to_ascii_lowercase();
    let required = required_field.to_ascii_lowercase();
    let field_pos = [
        format!("name=\"{required}\""),
        format!("name='{required}'"),
    ]
    .iter()
    .find_map(|needle| lower.find(needle))?;
    let form_start = lower[..field_pos].rfind("<form")?;
    let relative_end = lower[field_pos..].find("</form>")?;
    let form_end = field_pos + relative_end + "</form>".len();
    let form_html = &html[form_start..form_end];
    let open_end = form_html.find('>')?;
    let action = attribute_value(&form_html[..=open_end], "action").unwrap_or_default();
    let fields = input_fields(form_html);
    Some(HtmlForm {
        action: html_unescape(&action),
        fields,
        html: form_html.to_string(),
    })
}

fn input_fields(form_html: &str) -> Vec<(String, String)> {
    let lower = form_html.to_ascii_lowercase();
    let mut fields = Vec::new();
    let mut offset = 0;
    while let Some(relative_start) = lower[offset..].find("<input") {
        let start = offset + relative_start;
        let Some(relative_end) = lower[start..].find('>') else {
            break;
        };
        let end = start + relative_end + 1;
        let tag = &form_html[start..end];
        if let Some(name) = attribute_value(tag, "name") {
            let value = attribute_value(tag, "value").unwrap_or_default();
            fields.push((html_unescape(&name), html_unescape(&value)));
        }
        offset = end;
    }
    fields
}

fn attribute_value(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let name = name.to_ascii_lowercase();
    let mut search_from = 0;
    while let Some(relative_pos) = lower[search_from..].find(&name) {
        let pos = search_from + relative_pos;
        let before_ok = pos == 0
            || lower.as_bytes()[pos - 1].is_ascii_whitespace()
            || lower.as_bytes()[pos - 1] == b'<';
        let mut cursor = pos + name.len();
        while lower.as_bytes().get(cursor).is_some_and(u8::is_ascii_whitespace) {
            cursor += 1;
        }
        if before_ok && lower.as_bytes().get(cursor) == Some(&b'=') {
            cursor += 1;
            while lower.as_bytes().get(cursor).is_some_and(u8::is_ascii_whitespace) {
                cursor += 1;
            }
            let quote = *tag.as_bytes().get(cursor)?;
            if quote == b'\"' || quote == b'\'' {
                cursor += 1;
                let relative_end = tag[cursor..].find(quote as char)?;
                return Some(tag[cursor..cursor + relative_end].to_string());
            }
            let end = tag[cursor..]
                .find(|ch: char| ch.is_ascii_whitespace() || ch == '>')
                .map(|value| cursor + value)
                .unwrap_or(tag.len());
            return Some(tag[cursor..end].to_string());
        }
        search_from = pos + name.len();
    }
    None
}

fn preferred_action_value(form_html: &str) -> Option<String> {
    let lower = form_html.to_ascii_lowercase();
    for preferred in ["save_device", "dont_save"] {
        if lower.contains(&format!("value=\"{preferred}\""))
            || lower.contains(&format!("value='{preferred}'"))
        {
            return Some(preferred.to_string());
        }
    }
    None
}

fn upsert_field(fields: &mut Vec<(String, String)>, name: &str, value: &str) {
    fields.retain(|(existing, _)| existing != name);
    fields.push((name.to_string(), value.to_string()));
}

fn resolve_facebook_url(base: &Url, raw: &str) -> Result<Url> {
    let raw = html_unescape(raw.trim());
    let url = if raw.is_empty() {
        base.clone()
    } else {
        base.join(&raw)?
    };
    ensure_facebook_url(url.as_str())?;
    Ok(url)
}

fn ensure_facebook_url(raw: &str) -> Result<()> {
    let url = Url::parse(raw)?;
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    if url.scheme() != "https" || (host != "facebook.com" && !host.ends_with(".facebook.com")) {
        bail!("Facebook trả về địa chỉ chuyển hướng không an toàn.");
    }
    Ok(())
}

fn html_unescape(value: &str) -> String {
    value
        .replace("&amp;", "&")
        .replace("&#38;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
}

fn parse_cookie_header(raw: &str) -> BTreeMap<String, String> {
    let mut cookies = BTreeMap::new();
    for part in raw.split(|ch| ch == ';' || ch == '|') {
        let Some((name, value)) = part.trim().split_once('=') else {
            continue;
        };
        let name = name.trim();
        let value = value.trim();
        if !name.is_empty() && !value.is_empty() {
            cookies.insert(name.to_string(), value.to_string());
        }
    }
    cookies
}

fn merge_set_cookies(cookies: &mut BTreeMap<String, String>, headers: &HeaderMap) {
    for value in headers.get_all(SET_COOKIE).iter() {
        let Ok(raw) = value.to_str() else {
            continue;
        };
        let pair = raw.split(';').next().unwrap_or_default();
        let Some((name, value)) = pair.split_once('=') else {
            continue;
        };
        let name = name.trim();
        let value = value.trim();
        let lower = raw.to_ascii_lowercase();
        if value.is_empty()
            || lower.contains("max-age=0")
            || lower.contains("expires=thu, 01 jan 1970")
        {
            cookies.remove(name);
        } else if !name.is_empty() {
            cookies.insert(name.to_string(), value.to_string());
        }
    }
}

fn cookie_header(cookies: &BTreeMap<String, String>) -> String {
    cookies
        .iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join("; ")
}

fn has_login_cookies(cookies: &BTreeMap<String, String>) -> bool {
    cookies.get("c_user").is_some_and(|value| !value.is_empty())
        && cookies.get("xs").is_some_and(|value| !value.is_empty())
}

fn looks_like_facebook_cookie(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    lower.contains("c_user=") || lower.contains("xs=")
}

fn current_two_factor_code(value: &str) -> Result<String> {
    let compact = value
        .trim()
        .chars()
        .filter(|ch| *ch != ' ' && *ch != '-')
        .collect::<String>();
    if compact.len() == 6 && compact.bytes().all(|byte| byte.is_ascii_digit()) {
        return Ok(compact);
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("Đồng hồ hệ thống không hợp lệ")?
        .as_secs();
    totp_code_at(&compact, now)
}

fn totp_code_at(secret: &str, unix_seconds: u64) -> Result<String> {
    let key = decode_base32(secret)?;
    if key.is_empty() {
        bail!("Mã 2FA không hợp lệ.");
    }
    let counter = unix_seconds / 30;
    let digest = hmac_sha1(&key, &counter.to_be_bytes());
    let offset = (digest[19] & 0x0f) as usize;
    let binary = ((digest[offset] as u32 & 0x7f) << 24)
        | ((digest[offset + 1] as u32) << 16)
        | ((digest[offset + 2] as u32) << 8)
        | digest[offset + 3] as u32;
    Ok(format!("{:06}", binary % 1_000_000))
}

fn hmac_sha1(key: &[u8], message: &[u8]) -> [u8; 20] {
    const BLOCK_SIZE: usize = 64;
    let mut normalized_key = [0_u8; BLOCK_SIZE];
    if key.len() > BLOCK_SIZE {
        normalized_key[..20].copy_from_slice(&sha1_digest(key));
    } else {
        normalized_key[..key.len()].copy_from_slice(key);
    }

    let mut inner = Vec::with_capacity(BLOCK_SIZE + message.len());
    let mut outer = Vec::with_capacity(BLOCK_SIZE + 20);
    for byte in normalized_key {
        inner.push(byte ^ 0x36);
        outer.push(byte ^ 0x5c);
    }
    inner.extend_from_slice(message);
    outer.extend_from_slice(&sha1_digest(&inner));
    sha1_digest(&outer)
}

fn sha1_digest(data: &[u8]) -> [u8; 20] {
    let bit_len = (data.len() as u64) * 8;
    let mut padded = data.to_vec();
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());

    let mut h0 = 0x67452301_u32;
    let mut h1 = 0xefcdab89_u32;
    let mut h2 = 0x98badcfe_u32;
    let mut h3 = 0x10325476_u32;
    let mut h4 = 0xc3d2e1f0_u32;

    for chunk in padded.chunks_exact(64) {
        let mut words = [0_u32; 80];
        for (index, word) in words.iter_mut().take(16).enumerate() {
            let offset = index * 4;
            *word = u32::from_be_bytes([
                chunk[offset],
                chunk[offset + 1],
                chunk[offset + 2],
                chunk[offset + 3],
            ]);
        }
        for index in 16..80 {
            words[index] = (words[index - 3]
                ^ words[index - 8]
                ^ words[index - 14]
                ^ words[index - 16])
                .rotate_left(1);
        }

        let mut a = h0;
        let mut b = h1;
        let mut c = h2;
        let mut d = h3;
        let mut e = h4;
        for (index, word) in words.iter().enumerate() {
            let (f, k) = match index {
                0..=19 => ((b & c) | ((!b) & d), 0x5a827999),
                20..=39 => (b ^ c ^ d, 0x6ed9eba1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8f1bbcdc),
                _ => (b ^ c ^ d, 0xca62c1d6),
            };
            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(*word);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }
        h0 = h0.wrapping_add(a);
        h1 = h1.wrapping_add(b);
        h2 = h2.wrapping_add(c);
        h3 = h3.wrapping_add(d);
        h4 = h4.wrapping_add(e);
    }

    let mut digest = [0_u8; 20];
    for (index, value) in [h0, h1, h2, h3, h4].iter().enumerate() {
        digest[index * 4..index * 4 + 4].copy_from_slice(&value.to_be_bytes());
    }
    digest
}

fn decode_base32(value: &str) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    let mut buffer = 0_u32;
    let mut bits = 0_u8;
    for ch in value.chars().filter(|ch| !ch.is_ascii_whitespace() && *ch != '=') {
        let upper = ch.to_ascii_uppercase();
        let digit = match upper {
            'A'..='Z' => upper as u8 - b'A',
            '2'..='7' => upper as u8 - b'2' + 26,
            _ => bail!("Mã 2FA không hợp lệ."),
        };
        buffer = (buffer << 5) | digit as u32;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            output.push((buffer >> bits) as u8);
            buffer &= (1_u32 << bits) - 1;
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_supported_input_formats() {
        let full = FacebookCookieInput::parse(
            "123|password|JBSWY3DPEHPK3PXP|c_user=123; xs=abc",
        )
        .unwrap();
        assert_eq!(full.uid.as_deref(), Some("123"));
        assert_eq!(full.password.as_deref(), Some("password"));
        assert_eq!(full.two_fa.as_deref(), Some("JBSWY3DPEHPK3PXP"));
        assert_eq!(full.cookie.as_deref(), Some("c_user=123; xs=abc"));

        let credentials = FacebookCookieInput::parse("123|password|123456").unwrap();
        assert_eq!(credentials.cookie, None);

        let cookie = FacebookCookieInput::parse("c_user=123|xs=abc|fr=def").unwrap();
        assert_eq!(cookie.uid, None);
        assert_eq!(cookie.cookie.as_deref(), Some("c_user=123|xs=abc|fr=def"));
    }

    #[test]
    fn rejects_unknown_input_format() {
        assert!(FacebookCookieInput::parse("not-a-cookie").is_err());
        assert!(FacebookCookieInput::parse("uid|pass").is_err());
    }

    #[test]
    fn parses_login_form_and_hidden_fields() {
        let html = r#"
            <form method="post" action="/login/device-based/regular/login/?a=1&amp;b=2">
              <input type="hidden" name="lsd" value="token">
              <input name="email" value="">
              <input name="pass" value="">
            </form>
        "#;
        let form = find_form(html, "email").unwrap();
        assert_eq!(form.action, "/login/device-based/regular/login/?a=1&b=2");
        assert!(form.fields.contains(&("lsd".to_string(), "token".to_string())));
    }

    #[test]
    fn totp_matches_rfc_6238_sha1_vector_at_59_seconds() {
        assert_eq!(
            totp_code_at("GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ", 59).unwrap(),
            "287082"
        );
    }

    #[test]
    fn cookie_parser_accepts_semicolons_and_pipes() {
        let cookies = parse_cookie_header("c_user=123; xs=abc|fr=def");
        assert_eq!(cookies.get("c_user").map(String::as_str), Some("123"));
        assert_eq!(cookies.get("xs").map(String::as_str), Some("abc"));
        assert_eq!(cookies.get("fr").map(String::as_str), Some("def"));
        assert_eq!(cookie_header(&cookies), "c_user=123; fr=def; xs=abc");
    }

    #[test]
    fn only_allows_https_facebook_targets() {
        assert!(ensure_facebook_url("https://m.facebook.com/login").is_ok());
        assert!(ensure_facebook_url("http://m.facebook.com/login").is_err());
        assert!(ensure_facebook_url("https://example.com/login").is_err());
    }
}
